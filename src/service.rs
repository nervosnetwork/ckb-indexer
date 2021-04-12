use crate::indexer::{self, extract_raw_data, Indexer, Key, KeyPrefix, Value};
use crate::store::{IteratorDirection, RocksdbStore, Store};

use ckb_jsonrpc_types::{
    BlockNumber, BlockView, Capacity, CellOutput, JsonBytes, LocalNode, OutPoint, Script, Uint32,
    Uint64,
};
use ckb_types::{core, packed, prelude::*, H256};
use jsonrpc_core::{Error, IoHandler, Result};
use jsonrpc_core_client::RpcError;
use jsonrpc_derive::rpc;
use jsonrpc_http_server::{Server, ServerBuilder};
use jsonrpc_server_utils::cors::AccessControlAllowOrigin;
use jsonrpc_server_utils::hosts::DomainsValidation;
use log::{error, info, trace};
use rocksdb::{prelude::*, Direction, IteratorMode};
use serde::{Deserialize, Serialize};

use std::convert::TryInto;
use std::net::ToSocketAddrs;
use std::thread;
use std::time::Duration;

/// Have to use RocksdbStore instead of generic `Store` type here,
/// because some rpc need rocksdb snapshot funtion which has lifetime mark and is hard to wrap in a trait
pub struct Service {
    store: RocksdbStore,
    poll_interval: Duration,
    listen_address: String,
}

impl Service {
    pub fn new(store_path: &str, listen_address: &str, poll_interval: Duration) -> Self {
        let store = RocksdbStore::new(store_path);
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

    pub async fn poll(&self, rpc_client: gen_client::Client) {
        let indexer = Indexer::new(self.store.clone(), 100, 1000);
        // 0.37.0 and above supports hex format
        let use_hex_format = loop {
            match rpc_client.local_node_info().await {
                Ok(local_node_info) => {
                    break local_node_info.version > "0.36".to_owned();
                }
                Err(err) => {
                    // < 0.32.0 compatibility
                    if format!("#{}", err).contains("missing field") {
                        break false;
                    }
                    error!("cannot get local_node_info from ckb node: {}", err);
                    thread::sleep(self.poll_interval);
                }
            }
        };

        loop {
            if let Some((tip_number, tip_hash)) = indexer.tip().expect("get tip should be OK") {
                match get_block_by_number(&rpc_client, tip_number + 1, use_hex_format).await {
                    Ok(Some(block)) => {
                        if block.parent_hash() == tip_hash {
                            info!("append {}, {}", block.number(), block.hash());
                            indexer.append(&block).expect("append block should be OK");
                        } else {
                            info!("rollback {}, {}", tip_number, tip_hash);
                            indexer.rollback().expect("rollback block should be OK");
                        }
                    }
                    Ok(None) => {
                        trace!("no new block");
                        thread::sleep(self.poll_interval);
                    }
                    Err(err) => {
                        error!("cannot get block from ckb node, error: {}", err);
                        thread::sleep(self.poll_interval);
                    }
                }
            } else {
                match get_block_by_number(&rpc_client, 0, use_hex_format).await {
                    Ok(Some(block)) => indexer.append(&block).expect("append block should be OK"),
                    Ok(None) => {
                        error!("ckb node returns an empty genesis block");
                        thread::sleep(self.poll_interval);
                    }
                    Err(err) => {
                        error!("cannot get genesis block from ckb node, error: {}", err);
                        thread::sleep(self.poll_interval);
                    }
                }
            }
        }
    }
}

async fn get_block_by_number(
    rpc_client: &gen_client::Client,
    block_number: u64,
    use_hex_format: bool,
) -> std::result::Result<Option<core::BlockView>, RpcError> {
    if use_hex_format {
        rpc_client
            .get_block_by_number_with_verbosity(block_number.into(), 0.into())
            .await
            .map(|opt| {
                opt.map(|json_bytes| {
                    ckb_types::packed::Block::new_unchecked(json_bytes.into_bytes()).into_view()
                })
            })
    } else {
        rpc_client
            .get_block_by_number(block_number.into())
            .await
            .map(|opt| opt.map(Into::into))
    }
}

#[rpc(client)]
pub trait CkbRpc {
    #[rpc(name = "get_block_by_number")]
    fn get_block_by_number(&self, number: BlockNumber) -> Result<Option<BlockView>>;

    #[rpc(name = "get_block_by_number")]
    fn get_block_by_number_with_verbosity(
        &self,
        number: BlockNumber,
        verbosity: Uint32,
    ) -> Result<Option<JsonBytes>>;

    #[rpc(name = "local_node_info")]
    fn local_node_info(&self) -> Result<LocalNode>;
}

#[rpc(server)]
pub trait IndexerRpc {
    #[rpc(name = "get_tip")]
    fn get_tip(&self) -> Result<Option<Tip>>;

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

    #[rpc(name = "get_cells_capacity")]
    fn get_cells_capacity(&self, search_key: SearchKey) -> Result<Option<CellsCapacity>>;
}

#[derive(Deserialize)]
pub struct SearchKey {
    script: Script,
    script_type: ScriptType,
    filter: Option<SearchKeyFilter>,
}

#[derive(Deserialize, Default)]
pub struct SearchKeyFilter {
    script: Option<Script>,
    output_data_len_range: Option<[Uint64; 2]>,
    output_capacity_range: Option<[Uint64; 2]>,
    block_range: Option<[BlockNumber; 2]>,
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
pub struct Tip {
    block_hash: H256,
    block_number: BlockNumber,
}

#[derive(Serialize)]
pub struct CellsCapacity {
    capacity: Capacity,
    block_hash: H256,
    block_number: BlockNumber,
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

struct IndexerRpcImpl {
    store: RocksdbStore,
}

impl IndexerRpc for IndexerRpcImpl {
    fn get_tip(&self) -> Result<Option<Tip>> {
        let mut iter = self
            .store
            .iter(&[KeyPrefix::Header as u8 + 1], IteratorDirection::Reverse)
            .expect("iter Header should be OK");
        Ok(iter.next().map(|(key, _)| Tip {
            block_hash: packed::Byte32::from_slice(&key[9..])
                .expect("stored block key")
                .unpack(),
            block_number: core::BlockNumber::from_be_bytes(
                key[1..9].try_into().expect("stored block key"),
            )
            .into(),
        }))
    }

    fn get_cells(
        &self,
        search_key: SearchKey,
        order: Order,
        limit: Uint32,
        after_cursor: Option<JsonBytes>,
    ) -> Result<Pagination<Cell>> {
        let (prefix, from_key, direction, skip) = build_query_options(
            &search_key,
            KeyPrefix::CellLockScript,
            KeyPrefix::CellTypeScript,
            order,
            after_cursor,
        )?;
        let filter_script_type = match search_key.script_type {
            ScriptType::Lock => ScriptType::Type,
            ScriptType::Type => ScriptType::Lock,
        };
        let (
            filter_prefix,
            filter_output_data_len_range,
            filter_output_capacity_range,
            filter_block_range,
        ) = build_filter_options(search_key)?;
        let mode = IteratorMode::From(from_key.as_ref(), direction);
        let snapshot = self.store.inner().snapshot();
        let iter = snapshot.iterator(mode).skip(skip);

        let mut last_key = Vec::new();
        let cells = iter
            .take_while(|(key, _value)| key.starts_with(&prefix))
            .filter_map(|(key, value)| {
                let tx_hash = packed::Byte32::from_slice(&value).expect("stored tx hash");
                let index =
                    u32::from_be_bytes(key[key.len() - 4..].try_into().expect("stored index"));
                let out_point = packed::OutPoint::new(tx_hash, index);
                let (block_number, tx_index, output, output_data) = Value::parse_cell_value(
                    &snapshot
                        .get(Key::OutPoint(&out_point).into_vec())
                        .expect("get OutPoint should be OK")
                        .expect("stored OutPoint"),
                );

                if let Some(prefix) = filter_prefix.as_ref() {
                    match filter_script_type {
                        ScriptType::Lock => {
                            if !extract_raw_data(&output.lock())
                                .as_slice()
                                .starts_with(prefix)
                            {
                                return None;
                            }
                        }
                        ScriptType::Type => {
                            if output.type_().is_none()
                                || !extract_raw_data(&output.type_().to_opt().unwrap())
                                    .as_slice()
                                    .starts_with(prefix)
                            {
                                return None;
                            }
                        }
                    }
                }

                if let Some([r0, r1]) = filter_output_data_len_range {
                    if output_data.len() < r0 || output_data.len() >= r1 {
                        return None;
                    }
                }

                if let Some([r0, r1]) = filter_output_capacity_range {
                    let capacity: core::Capacity = output.capacity().unpack();
                    if capacity < r0 || capacity >= r1 {
                        return None;
                    }
                }

                if let Some([r0, r1]) = filter_block_range {
                    if block_number < r0 || block_number >= r1 {
                        return None;
                    }
                }

                last_key = key.to_vec();

                Some(Cell {
                    output: output.into(),
                    output_data: output_data.into(),
                    out_point: out_point.into(),
                    block_number: block_number.into(),
                    tx_index: tx_index.into(),
                })
            })
            .take(limit.value() as usize)
            .collect::<Vec<_>>();

        Ok(Pagination {
            objects: cells,
            last_cursor: JsonBytes::from_vec(last_key),
        })
    }

    fn get_transactions(
        &self,
        search_key: SearchKey,
        order: Order,
        limit: Uint32,
        after_cursor: Option<JsonBytes>,
    ) -> Result<Pagination<Tx>> {
        let (prefix, from_key, direction, skip) = build_query_options(
            &search_key,
            KeyPrefix::TxLockScript,
            KeyPrefix::TxTypeScript,
            order,
            after_cursor,
        )?;

        let (filter_script, filter_block_range) = if let Some(filter) = search_key.filter.as_ref() {
            if filter.output_data_len_range.is_some() {
                return Err(Error::invalid_params(
                    "doesn't support search_key.filter.output_data_len_range parameter",
                ));
            }
            if filter.output_capacity_range.is_some() {
                return Err(Error::invalid_params(
                    "doesn't support search_key.filter.output_capacity_range parameter",
                ));
            }
            let filter_script: Option<packed::Script> =
                filter.script.as_ref().map(|script| script.clone().into());
            let filter_block_range: Option<[core::BlockNumber; 2]> =
                filter.block_range.map(|r| [r[0].into(), r[1].into()]);
            (filter_script, filter_block_range)
        } else {
            (None, None)
        };

        let filter_script_type = match search_key.script_type {
            ScriptType::Lock => ScriptType::Type,
            ScriptType::Type => ScriptType::Lock,
        };

        let mode = IteratorMode::From(from_key.as_ref(), direction);
        let snapshot = self.store.inner().snapshot();
        let iter = snapshot.iterator(mode).skip(skip);

        let mut last_key = Vec::new();
        let txs = iter
            .take_while(|(key, _value)| key.starts_with(&prefix))
            .filter_map(|(key, value)| {
                let tx_hash = packed::Byte32::from_slice(&value).expect("stored tx hash");
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

                if let Some(filter_script) = filter_script.as_ref() {
                    match filter_script_type {
                        ScriptType::Lock => {
                            if snapshot
                                .get(
                                    Key::TxLockScript(
                                        &filter_script,
                                        block_number,
                                        tx_index,
                                        io_index,
                                        match io_type {
                                            IOType::Input => indexer::IOType::Input,
                                            IOType::Output => indexer::IOType::Output,
                                        },
                                    )
                                    .into_vec(),
                                )
                                .expect("get TxLockScript should be OK")
                                .is_none()
                            {
                                return None;
                            }
                        }
                        ScriptType::Type => {
                            if snapshot
                                .get(
                                    Key::TxTypeScript(
                                        &filter_script,
                                        block_number,
                                        tx_index,
                                        io_index,
                                        match io_type {
                                            IOType::Input => indexer::IOType::Input,
                                            IOType::Output => indexer::IOType::Output,
                                        },
                                    )
                                    .into_vec(),
                                )
                                .expect("get TxTypeScript should be OK")
                                .is_none()
                            {
                                return None;
                            }
                        }
                    }
                }

                if let Some([r0, r1]) = filter_block_range {
                    if block_number < r0 || block_number >= r1 {
                        return None;
                    }
                }

                last_key = key.to_vec();
                Some(Tx {
                    tx_hash: tx_hash.unpack(),
                    block_number: block_number.into(),
                    tx_index: tx_index.into(),
                    io_index: io_index.into(),
                    io_type,
                })
            })
            .take(limit.value() as usize)
            .collect::<Vec<_>>();

        Ok(Pagination {
            objects: txs,
            last_cursor: JsonBytes::from_vec(last_key),
        })
    }

    fn get_cells_capacity(&self, search_key: SearchKey) -> Result<Option<CellsCapacity>> {
        let (prefix, from_key, direction, skip) = build_query_options(
            &search_key,
            KeyPrefix::CellLockScript,
            KeyPrefix::CellTypeScript,
            Order::Asc,
            None,
        )?;
        let filter_script_type = match search_key.script_type {
            ScriptType::Lock => ScriptType::Type,
            ScriptType::Type => ScriptType::Lock,
        };
        let (
            filter_prefix,
            filter_output_data_len_range,
            filter_output_capacity_range,
            filter_block_range,
        ) = build_filter_options(search_key)?;
        let mode = IteratorMode::From(from_key.as_ref(), direction);
        let snapshot = self.store.inner().snapshot();
        let iter = snapshot.iterator(mode).skip(skip);

        let capacity: u64 = iter
            .take_while(|(key, _value)| key.starts_with(&prefix))
            .filter_map(|(key, value)| {
                let tx_hash = packed::Byte32::from_slice(value.as_ref()).expect("stored tx hash");
                let index =
                    u32::from_be_bytes(key[key.len() - 4..].try_into().expect("stored index"));
                let out_point = packed::OutPoint::new(tx_hash, index);
                let (block_number, _tx_index, output, output_data) = Value::parse_cell_value(
                    &snapshot
                        .get(Key::OutPoint(&out_point).into_vec())
                        .expect("get OutPoint should be OK")
                        .expect("stored OutPoint"),
                );

                if let Some(prefix) = filter_prefix.as_ref() {
                    match filter_script_type {
                        ScriptType::Lock => {
                            if !extract_raw_data(&output.lock())
                                .as_slice()
                                .starts_with(prefix)
                            {
                                return None;
                            }
                        }
                        ScriptType::Type => {
                            if output.type_().is_none()
                                || !extract_raw_data(&output.type_().to_opt().unwrap())
                                    .as_slice()
                                    .starts_with(prefix)
                            {
                                return None;
                            }
                        }
                    }
                }

                if let Some([r0, r1]) = filter_output_data_len_range {
                    if output_data.len() < r0 || output_data.len() >= r1 {
                        return None;
                    }
                }

                if let Some([r0, r1]) = filter_output_capacity_range {
                    let capacity: core::Capacity = output.capacity().unpack();
                    if capacity < r0 || capacity >= r1 {
                        return None;
                    }
                }

                if let Some([r0, r1]) = filter_block_range {
                    if block_number < r0 || block_number >= r1 {
                        return None;
                    }
                }

                Some(Unpack::<core::Capacity>::unpack(&output.capacity()).as_u64())
            })
            .sum();

        let tip_mode = IteratorMode::From(&[KeyPrefix::Header as u8 + 1], Direction::Reverse);
        let mut tip_iter = snapshot.iterator(tip_mode);
        Ok(tip_iter.next().map(|(key, _value)| CellsCapacity {
            capacity: capacity.into(),
            block_hash: packed::Byte32::from_slice(&key[9..])
                .expect("stored block key")
                .unpack(),
            block_number: core::BlockNumber::from_be_bytes(
                key[1..9].try_into().expect("stored block key"),
            )
            .into(),
        }))
    }
}

const MAX_PREFIX_SEARCH_SIZE: usize = u16::max_value() as usize;

// a helper fn to build query options from search paramters, returns prefix, from_key, direction and skip offset
fn build_query_options(
    search_key: &SearchKey,
    lock_prefix: KeyPrefix,
    type_prefix: KeyPrefix,
    order: Order,
    after_cursor: Option<JsonBytes>,
) -> Result<(Vec<u8>, Vec<u8>, Direction, usize)> {
    let mut prefix = match search_key.script_type {
        ScriptType::Lock => vec![lock_prefix as u8],
        ScriptType::Type => vec![type_prefix as u8],
    };
    let script: packed::Script = search_key.script.clone().into();
    let args_len = script.args().len();
    if args_len > MAX_PREFIX_SEARCH_SIZE {
        return Err(Error::invalid_params(format!(
            "search_key.script.args len should be less than {}",
            MAX_PREFIX_SEARCH_SIZE
        )));
    }
    prefix.extend_from_slice(extract_raw_data(&script).as_slice());

    let (from_key, direction, skip) = match order {
        Order::Asc => after_cursor.map_or_else(
            || (prefix.clone(), Direction::Forward, 0),
            |json_bytes| (json_bytes.as_bytes().into(), Direction::Forward, 1),
        ),
        Order::Desc => after_cursor.map_or_else(
            || {
                (
                    [
                        prefix.clone(),
                        vec![0xff; MAX_PREFIX_SEARCH_SIZE - args_len],
                    ]
                    .concat(),
                    Direction::Reverse,
                    0,
                )
            },
            |json_bytes| (json_bytes.as_bytes().into(), Direction::Reverse, 1),
        ),
    };

    Ok((prefix, from_key, direction, skip))
}

// a helper fn to build filter options from search paramters, returns prefix, output_data_len_range, output_capacity_range and block_range
fn build_filter_options(
    search_key: SearchKey,
) -> Result<(
    Option<Vec<u8>>,
    Option<[usize; 2]>,
    Option<[core::Capacity; 2]>,
    Option<[core::BlockNumber; 2]>,
)> {
    let SearchKey {
        script: _,
        script_type: _,
        filter,
    } = search_key;
    let filter = filter.unwrap_or_default();
    let filter_script_prefix = if let Some(script) = filter.script {
        let script: packed::Script = script.into();
        if script.args().len() > MAX_PREFIX_SEARCH_SIZE {
            return Err(Error::invalid_params(format!(
                "search_key.filter.script.args len should be less than {}",
                MAX_PREFIX_SEARCH_SIZE
            )));
        }
        let mut prefix = Vec::new();
        prefix.extend_from_slice(extract_raw_data(&script).as_slice());
        Some(prefix)
    } else {
        None
    };

    let filter_output_data_len_range = filter.output_data_len_range.map(|[r0, r1]| {
        [
            Into::<u64>::into(r0) as usize,
            Into::<u64>::into(r1) as usize,
        ]
    });
    let filter_output_capacity_range = filter.output_capacity_range.map(|[r0, r1]| {
        [
            core::Capacity::shannons(r0.into()),
            core::Capacity::shannons(r1.into()),
        ]
    });
    let filter_block_range = filter.block_range.map(|r| [r[0].into(), r[1].into()]);

    Ok((
        filter_script_prefix,
        filter_output_data_len_range,
        filter_output_capacity_range,
        filter_block_range,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::RocksdbStore;
    use ckb_types::{
        bytes::Bytes,
        core::{
            capacity_bytes, BlockBuilder, Capacity, HeaderBuilder, ScriptHashType,
            TransactionBuilder,
        },
        packed::{CellInput, CellOutputBuilder, OutPoint, Script, ScriptBuilder},
        H256,
    };
    use tempfile;

    fn new_store(prefix: &str) -> RocksdbStore {
        let tmp_dir = tempfile::Builder::new().prefix(prefix).tempdir().unwrap();
        RocksdbStore::new(tmp_dir.path().to_str().unwrap())
        // Indexer::new(store, 10, 1)
    }

    #[test]
    fn rpc() {
        let store = new_store("rpc");
        let indexer = Indexer::new(store.clone(), 10, 100);
        let rpc = IndexerRpcImpl { store };

        // setup test data
        let lock_script1 = ScriptBuilder::default()
            .code_hash(H256(rand::random()).pack())
            .hash_type(ScriptHashType::Data.into())
            .args(Bytes::from(b"lock_script1".to_vec()).pack())
            .build();

        let lock_script2 = ScriptBuilder::default()
            .code_hash(H256(rand::random()).pack())
            .hash_type(ScriptHashType::Type.into())
            .args(Bytes::from(b"lock_script2".to_vec()).pack())
            .build();

        let type_script1 = ScriptBuilder::default()
            .code_hash(H256(rand::random()).pack())
            .hash_type(ScriptHashType::Data.into())
            .args(Bytes::from(b"type_script1".to_vec()).pack())
            .build();

        let type_script2 = ScriptBuilder::default()
            .code_hash(H256(rand::random()).pack())
            .hash_type(ScriptHashType::Type.into())
            .args(Bytes::from(b"type_script2".to_vec()).pack())
            .build();

        let cellbase0 = TransactionBuilder::default()
            .input(CellInput::new_cellbase_input(0))
            .witness(Script::default().into_witness())
            .output(
                CellOutputBuilder::default()
                    .capacity(capacity_bytes!(1000).pack())
                    .lock(lock_script1.clone())
                    .build(),
            )
            .output_data(Default::default())
            .build();

        let tx00 = TransactionBuilder::default()
            .output(
                CellOutputBuilder::default()
                    .capacity(capacity_bytes!(1000).pack())
                    .lock(lock_script1.clone())
                    .type_(Some(type_script1.clone()).pack())
                    .build(),
            )
            .output_data(Default::default())
            .build();

        let tx01 = TransactionBuilder::default()
            .output(
                CellOutputBuilder::default()
                    .capacity(capacity_bytes!(2000).pack())
                    .lock(lock_script2.clone())
                    .type_(Some(type_script2.clone()).pack())
                    .build(),
            )
            .output_data(Default::default())
            .build();

        let block0 = BlockBuilder::default()
            .transaction(cellbase0)
            .transaction(tx00.clone())
            .transaction(tx01.clone())
            .header(HeaderBuilder::default().number(0.pack()).build())
            .build();

        indexer.append(&block0).unwrap();

        let (mut pre_tx0, mut pre_tx1, mut pre_block) = (tx00, tx01, block0);
        let total_blocks = 255;
        for i in 1..total_blocks {
            let cellbase = TransactionBuilder::default()
                .input(CellInput::new_cellbase_input(i + 1))
                .witness(Script::default().into_witness())
                .output(
                    CellOutputBuilder::default()
                        .capacity(capacity_bytes!(1000).pack())
                        .lock(lock_script1.clone())
                        .build(),
                )
                .output_data(Default::default())
                .build();

            pre_tx0 = TransactionBuilder::default()
                .input(CellInput::new(OutPoint::new(pre_tx0.hash(), 0), 0))
                .output(
                    CellOutputBuilder::default()
                        .capacity(capacity_bytes!(1000).pack())
                        .lock(lock_script1.clone())
                        .type_(Some(type_script1.clone()).pack())
                        .build(),
                )
                .output_data(Default::default())
                .build();

            pre_tx1 = TransactionBuilder::default()
                .input(CellInput::new(OutPoint::new(pre_tx1.hash(), 0), 0))
                .output(
                    CellOutputBuilder::default()
                        .capacity(capacity_bytes!(2000).pack())
                        .lock(lock_script2.clone())
                        .type_(Some(type_script2.clone()).pack())
                        .build(),
                )
                .output_data(Default::default())
                .build();

            pre_block = BlockBuilder::default()
                .transaction(cellbase)
                .transaction(pre_tx0.clone())
                .transaction(pre_tx1.clone())
                .header(
                    HeaderBuilder::default()
                        .number((pre_block.number() + 1).pack())
                        .parent_hash(pre_block.hash())
                        .build(),
                )
                .build();

            indexer.append(&pre_block).unwrap();
        }

        // test get_tip rpc
        let tip = rpc.get_tip().unwrap().unwrap();
        assert_eq!(Unpack::<H256>::unpack(&pre_block.hash()), tip.block_hash);
        assert_eq!(pre_block.number(), tip.block_number.value());

        // test get_cells rpc
        let cells_page_1 = rpc
            .get_cells(
                SearchKey {
                    script: lock_script1.clone().into(),
                    script_type: ScriptType::Lock,
                    filter: None,
                },
                Order::Asc,
                150.into(),
                None,
            )
            .unwrap();
        let cells_page_2 = rpc
            .get_cells(
                SearchKey {
                    script: lock_script1.clone().into(),
                    script_type: ScriptType::Lock,
                    filter: None,
                },
                Order::Asc,
                150.into(),
                Some(cells_page_1.last_cursor),
            )
            .unwrap();

        assert_eq!(
            total_blocks as usize + 1,
            cells_page_1.objects.len() + cells_page_2.objects.len(),
            "total size should be cellbase cells count + 1 (last block live cell)"
        );

        let desc_cells_page_1 = rpc
            .get_cells(
                SearchKey {
                    script: lock_script1.clone().into(),
                    script_type: ScriptType::Lock,
                    filter: None,
                },
                Order::Desc,
                150.into(),
                None,
            )
            .unwrap();

        let desc_cells_page_2 = rpc
            .get_cells(
                SearchKey {
                    script: lock_script1.clone().into(),
                    script_type: ScriptType::Lock,
                    filter: None,
                },
                Order::Desc,
                150.into(),
                Some(desc_cells_page_1.last_cursor),
            )
            .unwrap();

        assert_eq!(
            total_blocks as usize + 1,
            desc_cells_page_1.objects.len() + desc_cells_page_2.objects.len(),
            "total size should be cellbase cells count + 1 (last block live cell)"
        );
        assert_eq!(
            desc_cells_page_1.objects.first().unwrap().out_point,
            cells_page_2.objects.last().unwrap().out_point
        );

        let filter_cells_page_1 = rpc
            .get_cells(
                SearchKey {
                    script: lock_script1.clone().into(),
                    script_type: ScriptType::Lock,
                    filter: Some(SearchKeyFilter {
                        script: None,
                        output_data_len_range: None,
                        output_capacity_range: None,
                        block_range: Some([100.into(), 200.into()]),
                    }),
                },
                Order::Asc,
                60.into(),
                None,
            )
            .unwrap();

        let filter_cells_page_2 = rpc
            .get_cells(
                SearchKey {
                    script: lock_script1.clone().into(),
                    script_type: ScriptType::Lock,
                    filter: Some(SearchKeyFilter {
                        script: None,
                        output_data_len_range: None,
                        output_capacity_range: None,
                        block_range: Some([100.into(), 200.into()]),
                    }),
                },
                Order::Asc,
                60.into(),
                Some(filter_cells_page_1.last_cursor),
            )
            .unwrap();

        assert_eq!(
            100,
            filter_cells_page_1.objects.len() + filter_cells_page_2.objects.len(),
            "total size should be filtered cellbase cells (100~199)"
        );

        // test get_transactions rpc
        let txs_page_1 = rpc
            .get_transactions(
                SearchKey {
                    script: lock_script1.clone().into(),
                    script_type: ScriptType::Lock,
                    filter: None,
                },
                Order::Asc,
                500.into(),
                None,
            )
            .unwrap();
        let txs_page_2 = rpc
            .get_transactions(
                SearchKey {
                    script: lock_script1.clone().into(),
                    script_type: ScriptType::Lock,
                    filter: None,
                },
                Order::Asc,
                500.into(),
                Some(txs_page_1.last_cursor),
            )
            .unwrap();

        assert_eq!(total_blocks as usize * 3 - 1, txs_page_1.objects.len() + txs_page_2.objects.len(), "total size should be cellbase tx count + total_block * 2 - 1 (genesis block only has one tx)");

        let desc_txs_page_1 = rpc
            .get_transactions(
                SearchKey {
                    script: lock_script1.clone().into(),
                    script_type: ScriptType::Lock,
                    filter: None,
                },
                Order::Desc,
                500.into(),
                None,
            )
            .unwrap();
        let desc_txs_page_2 = rpc
            .get_transactions(
                SearchKey {
                    script: lock_script1.clone().into(),
                    script_type: ScriptType::Lock,
                    filter: None,
                },
                Order::Desc,
                500.into(),
                Some(desc_txs_page_1.last_cursor),
            )
            .unwrap();

        assert_eq!(total_blocks as usize * 3 - 1, desc_txs_page_1.objects.len() + desc_txs_page_2.objects.len(), "total size should be cellbase tx count + total_block * 2 - 1 (genesis block only has one tx)");
        assert_eq!(
            desc_txs_page_1.objects.first().unwrap().tx_hash,
            txs_page_2.objects.last().unwrap().tx_hash
        );

        let filter_txs_page_1 = rpc
            .get_transactions(
                SearchKey {
                    script: lock_script1.clone().into(),
                    script_type: ScriptType::Lock,
                    filter: Some(SearchKeyFilter {
                        script: None,
                        output_data_len_range: None,
                        output_capacity_range: None,
                        block_range: Some([100.into(), 200.into()]),
                    }),
                },
                Order::Asc,
                200.into(),
                None,
            )
            .unwrap();

        let filter_txs_page_2 = rpc
            .get_transactions(
                SearchKey {
                    script: lock_script1.clone().into(),
                    script_type: ScriptType::Lock,
                    filter: Some(SearchKeyFilter {
                        script: None,
                        output_data_len_range: None,
                        output_capacity_range: None,
                        block_range: Some([100.into(), 200.into()]),
                    }),
                },
                Order::Asc,
                200.into(),
                Some(filter_txs_page_1.last_cursor),
            )
            .unwrap();

        assert_eq!(
            300,
            filter_txs_page_1.objects.len() + filter_txs_page_2.objects.len(),
            "total size should be filtered blocks count * 3 (100~199 * 3)"
        );

        // test get_cells_capacity rpc
        let capacity = rpc
            .get_cells_capacity(SearchKey {
                script: lock_script1.clone().into(),
                script_type: ScriptType::Lock,
                filter: None,
            })
            .unwrap()
            .unwrap();

        assert_eq!(
            1000 * 100000000 * (total_blocks + 1),
            capacity.capacity.value(),
            "cellbases + last block live cell"
        );

        let capacity = rpc
            .get_cells_capacity(SearchKey {
                script: lock_script2.clone().into(),
                script_type: ScriptType::Lock,
                filter: None,
            })
            .unwrap()
            .unwrap();

        assert_eq!(
            2000 * 100000000,
            capacity.capacity.value(),
            "last block live cell"
        );
    }
}
