use crate::indexer::Indexer;
use crate::store::{RocksdbStore, Store};
use ckb_jsonrpc_types::{BlockNumber, BlockView, HeaderView};
use ckb_types::H256;
use futures::future::Future;
use jsonrpc_core::Result;
use jsonrpc_derive::rpc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

pub struct Service {
    indexer: Indexer<RocksdbStore>,
    poll_interval: Duration,
    running: AtomicBool,
}

impl Service {
    pub fn new(store_path: &str, poll_interval: Duration) -> Self {
        let indexer = Indexer::new(RocksdbStore::new(store_path), 100);
        // let client = connect(rpc_url).wait().expect("connect");
        Self {
            indexer,
            poll_interval,
            running: AtomicBool::new(false),
        }
    }

    // pub fn start<S: ToString>(self, thread_name: Option<S>) {
    //     let mut thread_builder = thread::Builder::new();
    //     if let Some(name) = thread_name {
    //         thread_builder = thread_builder.name(name.to_string());
    //     }

    //     thread_builder
    //         .spawn(move || loop {
    //             self.poll();
    //         })
    //         .expect("start service failed");
    // }

    pub fn running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Release);
    }

    pub fn poll(&self, rpc_client: gen_client::Client) {
        self.running.store(true, Ordering::Release);
        loop {
            if !self.running.load(Ordering::Acquire) {
                break;
            }
            // TODO log error
            if let Some((tip_number, tip_hash)) = self.indexer.tip().unwrap() {
                if let Ok(Some(block)) = rpc_client
                    .get_block_by_number((tip_number + 1).into())
                    .wait()
                {
                    let block: ckb_types::core::BlockView = block.into();
                    if block.parent_hash() == tip_hash {
                        info!("append {:?}", block.number());
                        self.indexer.append(&block).unwrap();
                    } else {
                        info!("rollback");
                        self.indexer.rollback().unwrap();
                    }
                } else {
                    thread::sleep(self.poll_interval);
                }
            } else {
                if let Ok(Some(block)) = rpc_client.get_block_by_number(0u64.into()).wait() {
                    self.indexer.append(&block.into()).unwrap();
                }
            }
        }
    }
}

#[rpc]
pub trait Rpc {
    #[rpc(name = "get_block")]
    fn get_block(&self, _hash: H256) -> Result<Option<BlockView>>;

    #[rpc(name = "get_block_by_number")]
    fn get_block_by_number(&self, _number: BlockNumber) -> Result<Option<BlockView>>;

    #[rpc(name = "get_tip_header")]
    fn get_tip_header(&self) -> Result<HeaderView>;
}
