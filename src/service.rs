use crate::indexer::{Indexer, Key, KeyPrefix, Value};
use crate::store::{IteratorDirection, Store};
use ckb_jsonrpc_types::{BlockNumber, BlockView, CellOutput, JsonBytes, OutPoint, Script, Uint32};
use ckb_types::{packed, prelude::*, H256};
use futures::future::Future;
use jsonrpc_core::{Error, IoHandler, Result};
use jsonrpc_derive::rpc;
use jsonrpc_http_server::{Server, ServerBuilder};
use jsonrpc_server_utils::cors::AccessControlAllowOrigin;
use jsonrpc_server_utils::hosts::DomainsValidation;
use log::info;
use serde::{Deserialize, Serialize};
use std::convert::TryInto;
use std::net::ToSocketAddrs;
use std::thread;
use std::time::Duration;

pub struct Service<S> {
    store: S,
    poll_interval: Duration,
    listen_address: String,
}

impl<S: Store + Clone + Send + Sync + 'static> Service<S> {
    pub fn new(store_path: &str, listen_address: &str, poll_interval: Duration) -> Self {
        let store = S::new(store_path);
        Self {
            store,
            listen_address: listen_address.to_string(),
            poll_interval,
        }
    }

    pub fn start(&self) -> Server {
        let mut io_handler = IoHandler::new();
        let rpc_impl = IndexerRpcImpl {
            store: self.store.clone(),
        };
        io_handler.extend_with(rpc_impl.to_delegate());

        ServerBuilder::new(io_handler)
            .cors(DomainsValidation::AllowOnly(vec![
                AccessControlAllowOrigin::Null,
                AccessControlAllowOrigin::Any,
            ]))
            .health_api(("/ping", "ping"))
            .start_http(
                &self
                    .listen_address
                    .to_socket_addrs()
                    .expect("config listen_address parsed")
                    .next()
                    .expect("config listen_address parsed"),
            )
            .expect("Start Jsonrpc HTTP service")
    }

    pub fn poll(&self, rpc_client: gen_client::Client) {
        let indexer = Indexer::new(self.store.clone(), 100, 1000);
        loop {
            if let Some((tip_number, tip_hash)) = indexer.tip().unwrap() {
                if let Ok(Some(block)) = rpc_client
                    .get_block_by_number((tip_number + 1).into())
                    .wait()
                {
                    let block: ckb_types::core::BlockView = block.into();
                    if block.parent_hash() == tip_hash {
                        info!("append {}, {}", block.number(), block.hash());
                        indexer.append(&block).unwrap();
                    } else {
                        info!("rollback {}, {}", tip_number, tip_hash);
                        indexer.rollback().unwrap();
                    }
                } else {
                    thread::sleep(self.poll_interval);
                }
            } else {
                if let Ok(Some(block)) = rpc_client.get_block_by_number(0u64.into()).wait() {
                    indexer.append(&block.into()).unwrap();
                }
            }
        }
    }
}

#[rpc(client)]
pub trait CkbRpc {
    #[rpc(name = "get_block_by_number")]
    fn get_block_by_number(&self, _number: BlockNumber) -> Result<Option<BlockView>>;
}

#[rpc(server)]
pub trait IndexerRpc {
    #[rpc(name = "get_cells")]
    fn get_cells(
        &self,
        search_key: SearchKey,
        order: Order,
        limit: Uint32,
        after: Option<JsonBytes>,
    ) -> Result<Pagination<Cell>>;

    #[rpc(name = "get_transactions")]
    fn get_transactions(
        &self,
        search_key: SearchKey,
        order: Order,
        limit: Uint32,
        after: Option<JsonBytes>,
    ) -> Result<Pagination<Tx>>;
}

#[derive(Deserialize)]
pub struct SearchKey {
    script: Script,
    script_type: ScriptType,
    args_len: Option<Uint32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptType {
    Lock,
    Type,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    Desc,
    Asc,
}

#[derive(Serialize)]
pub struct Cell {
    output: CellOutput,
    output_data: JsonBytes,
    out_point: OutPoint,
    block_number: BlockNumber,
    tx_index: Uint32,
}

#[derive(Serialize)]
pub struct Tx {
    tx_hash: H256,
    block_number: BlockNumber,
    tx_index: Uint32,
    io_index: Uint32,
    io_type: IOType,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IOType {
    Input,
    Output,
}

#[derive(Serialize)]
pub struct Pagination<T> {
    objects: Vec<T>,
    last_cursor: JsonBytes,
}

struct IndexerRpcImpl<S> {
    store: S,
}

impl<S: Store + Send + Sync + 'static> IndexerRpc for IndexerRpcImpl<S> {
    fn get_cells(
        &self,
        search_key: SearchKey,
        order: Order,
        limit: Uint32,
        after_cursor: Option<JsonBytes>,
    ) -> Result<Pagination<Cell>> {
        let mut prefix = match search_key.script_type {
            ScriptType::Lock => vec![KeyPrefix::CellLockScript as u8],
            ScriptType::Type => vec![KeyPrefix::CellTypeScript as u8],
        };
        let script: packed::Script = search_key.script.into();
        let args_len = search_key
            .args_len
            .map_or_else(|| script.args().len(), |args_len| args_len.value() as usize);
        if args_len < script.args().len() {
            return Err(Error::invalid_params(
                "args_len should be greater than or equal to script.args.len",
            ));
        }
        if args_len > u16::max_value() as usize {
            return Err(Error::invalid_params("args_len should be less than 65535"));
        }
        prefix.extend_from_slice(script.code_hash().as_slice());
        prefix.extend_from_slice(script.hash_type().as_slice());
        prefix.extend_from_slice(&(args_len as u32).to_le_bytes());
        prefix.extend_from_slice(&script.args().raw_data());

        let remain_args_len = args_len - script.args().len();
        let (from_key, direction, skip) = match order {
            Order::Asc => {
                (after_cursor.map_or_else(
                    || (prefix.clone(), IteratorDirection::Forward, 0),
                    |json_bytes| (json_bytes.as_bytes().into(), IteratorDirection::Forward, 1),
                ))
            }
            Order::Desc => {
                (after_cursor.map_or_else(
                    // 16 is BlockNumber + TxIndex + OutputIndex length
                    || {
                        (
                            [prefix.clone(), vec![0xff; remain_args_len + 16]].concat(),
                            IteratorDirection::Reverse,
                            0,
                        )
                    },
                    |json_bytes| (json_bytes.as_bytes().into(), IteratorDirection::Reverse, 1),
                ))
            }
        };

        let iter = self
            .store
            .iter(&from_key, direction)
            .expect("indexer store should be OK")
            .skip(skip);

        let kvs = iter
            .take(limit.value() as usize)
            .take_while(|(key, _value)| key.starts_with(&prefix))
            .collect::<Vec<_>>();

        let cells = kvs
            .iter()
            .map(|(key, value)| {
                let tx_hash = packed::Byte32::from_slice(value).expect("stored tx hash");
                let index =
                    u32::from_be_bytes(key[key.len() - 4..].try_into().expect("stored index"));
                let out_point = packed::OutPoint::new(tx_hash, index);
                let (block_number, tx_index, output, output_data) = Value::parse_cell_value(
                    &self
                        .store
                        .get(Key::OutPoint(&out_point).into_vec())
                        .unwrap()
                        .unwrap(),
                );
                Cell {
                    output: output.into(),
                    output_data: output_data.into(),
                    out_point: out_point.into(),
                    block_number: block_number.into(),
                    tx_index: tx_index.into(),
                }
            })
            .collect::<Vec<Cell>>();

        let last_cursor = kvs.last().map_or_else(
            || JsonBytes::default(),
            |(last_key, _last_value)| JsonBytes::from_vec(last_key.clone().into()),
        );

        Ok(Pagination {
            objects: cells,
            last_cursor,
        })
    }

    fn get_transactions(
        &self,
        search_key: SearchKey,
        order: Order,
        limit: Uint32,
        after_cursor: Option<JsonBytes>,
    ) -> Result<Pagination<Tx>> {
        let mut prefix = match search_key.script_type {
            ScriptType::Lock => vec![KeyPrefix::TxLockScript as u8],
            ScriptType::Type => vec![KeyPrefix::TxTypeScript as u8],
        };
        let script: packed::Script = search_key.script.into();
        let args_len = search_key
            .args_len
            .map_or_else(|| script.args().len(), |args_len| args_len.value() as usize);
        if args_len < script.args().len() {
            return Err(Error::invalid_params(
                "args_len should be greater than or equal to script.args.len",
            ));
        }
        if args_len > u16::max_value() as usize {
            return Err(Error::invalid_params("args_len should be less than 65535"));
        }
        prefix.extend_from_slice(script.code_hash().as_slice());
        prefix.extend_from_slice(script.hash_type().as_slice());
        prefix.extend_from_slice(&(args_len as u32).to_le_bytes());
        prefix.extend_from_slice(&script.args().raw_data());

        let remain_args_len = args_len - script.args().len();
        let (from_key, direction, skip) = match order {
            Order::Asc => {
                (after_cursor.map_or_else(
                    || (prefix.clone(), IteratorDirection::Forward, 0),
                    |json_bytes| (json_bytes.as_bytes().into(), IteratorDirection::Forward, 1),
                ))
            }
            Order::Desc => {
                (after_cursor.map_or_else(
                    // 17 is BlockNumber + TxIndex + IOIndex + IOType length
                    || {
                        (
                            [prefix.clone(), vec![0xff; remain_args_len + 17]].concat(),
                            IteratorDirection::Reverse,
                            0,
                        )
                    },
                    |json_bytes| (json_bytes.as_bytes().into(), IteratorDirection::Reverse, 1),
                ))
            }
        };

        let iter = self
            .store
            .iter(&from_key, direction)
            .expect("indexer store should be OK")
            .skip(skip);

        let kvs = iter
            .take_while(|(key, _value)| key.starts_with(&prefix))
            .take(limit.value() as usize)
            .collect::<Vec<_>>();

        let txs = kvs
            .iter()
            .map(|(key, value)| {
                let tx_hash = packed::Byte32::from_slice(value).expect("stored tx hash");
                let block_number = u64::from_be_bytes(
                    key[key.len() - 17..key.len() - 9]
                        .try_into()
                        .expect("stored block_number"),
                );
                let tx_index = u32::from_be_bytes(
                    key[key.len() - 9..key.len() - 5]
                        .try_into()
                        .expect("stored tx_index"),
                );
                let io_index = u32::from_be_bytes(
                    key[key.len() - 5..key.len() - 1]
                        .try_into()
                        .expect("stored io_index"),
                );
                let io_type = if *key.last().expect("stored io_type") == 0 {
                    IOType::Input
                } else {
                    IOType::Output
                };

                Tx {
                    tx_hash: tx_hash.unpack(),
                    block_number: block_number.into(),
                    tx_index: tx_index.into(),
                    io_index: io_index.into(),
                    io_type,
                }
            })
            .collect::<Vec<_>>();

        let last_cursor = kvs.last().map_or_else(
            || JsonBytes::default(),
            |(last_key, _last_value)| JsonBytes::from_vec(last_key.clone().into()),
        );

        Ok(Pagination {
            objects: txs,
            last_cursor,
        })
    }
}
