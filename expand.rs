#![feature(prelude_import)]
#[prelude_import]
use std::prelude::v1::*;
#[macro_use]
extern crate std;
pub mod indexer {
    use crate::store::{Batch, Error as StoreError, IteratorDirection, Store};
    use ckb_types::{
        core::{BlockNumber, BlockView},
        packed::{Byte32, Bytes, CellOutput, OutPoint, Script},
        prelude::*,
    };
    use std::collections::HashMap;
    use std::convert::TryInto;
    pub type TxIndex = u32;
    pub type OutputIndex = u32;
    pub type IOIndex = u32;
    pub enum IOType {
        Input,
        Output,
    }
    pub const SCRIPT_SERIALIZE_OFFSET: usize = 16;
    #[doc = " +--------------+--------------------+--------------------------+"]
    #[doc = " | KeyPrefix::  | Key::              | Value::                  |"]
    #[doc = " +--------------+--------------------+--------------------------+"]
    #[doc = " | 0            | OutPoint           | Cell                     |"]
    #[doc = " | 32           | ConsumedOutPoint   | Cell                     | * rollback and prune"]
    #[doc = " | 64           | CellLockScript     | TxHash                   |"]
    #[doc = " | 96           | CellTypeScript     | TxHash                   |"]
    #[doc = " | 128          | TxLockScript       | TxHash                   |"]
    #[doc = " | 160          | TxTypeScript       | TxHash                   |"]
    #[doc = " | 192          | TxHash             | TransactionInputs        | * rollback and prune"]
    #[doc = " | 224          | Header             | Transactions             |"]
    #[doc = " +--------------+--------------------+--------------------------+"]
    pub enum Key<'a> {
        OutPoint(&'a OutPoint),
        ConsumedOutPoint(BlockNumber, &'a OutPoint),
        CellLockScript(&'a Script, BlockNumber, TxIndex, OutputIndex),
        CellTypeScript(&'a Script, BlockNumber, TxIndex, OutputIndex),
        TxLockScript(&'a Script, BlockNumber, TxIndex, IOIndex, IOType),
        TxTypeScript(&'a Script, BlockNumber, TxIndex, IOIndex, IOType),
        TxHash(&'a Byte32),
        Header(BlockNumber, &'a Byte32),
    }
    pub enum Value<'a> {
        Cell(BlockNumber, TxIndex, &'a CellOutput, &'a Bytes),
        TxHash(&'a Byte32),
        TransactionInputs(Vec<OutPoint>),
        Transactions(Vec<(Byte32, u32)>),
    }
    #[repr(u8)]
    pub enum KeyPrefix {
        OutPoint = 0,
        ConsumedOutPoint = 32,
        CellLockScript = 64,
        CellTypeScript = 96,
        TxLockScript = 128,
        TxTypeScript = 160,
        TxHash = 192,
        Header = 224,
    }
    impl<'a> Key<'a> {
        pub fn into_vec(self) -> Vec<u8> {
            self.into()
        }
    }
    impl<'a> Into<Vec<u8>> for Key<'a> {
        fn into(self) -> Vec<u8> {
            let mut encoded = Vec::new();
            match self {
                Key::OutPoint(out_point) => {
                    encoded.push(KeyPrefix::OutPoint as u8);
                    encoded.extend_from_slice(out_point.as_slice());
                }
                Key::ConsumedOutPoint(block_number, out_point) => {
                    encoded.push(KeyPrefix::ConsumedOutPoint as u8);
                    encoded.extend_from_slice(&block_number.to_be_bytes());
                    encoded.extend_from_slice(out_point.as_slice());
                }
                Key::CellLockScript(script, block_number, tx_index, output_index) => {
                    encoded.push(KeyPrefix::CellLockScript as u8);
                    encoded.extend_from_slice(&script.as_slice()[SCRIPT_SERIALIZE_OFFSET..]);
                    encoded.extend_from_slice(&block_number.to_be_bytes());
                    encoded.extend_from_slice(&tx_index.to_be_bytes());
                    encoded.extend_from_slice(&output_index.to_be_bytes());
                }
                Key::CellTypeScript(script, block_number, tx_index, output_index) => {
                    encoded.push(KeyPrefix::CellTypeScript as u8);
                    encoded.extend_from_slice(&script.as_slice()[SCRIPT_SERIALIZE_OFFSET..]);
                    encoded.extend_from_slice(&block_number.to_be_bytes());
                    encoded.extend_from_slice(&tx_index.to_be_bytes());
                    encoded.extend_from_slice(&output_index.to_be_bytes());
                }
                Key::TxLockScript(script, block_number, tx_index, io_index, io_type) => {
                    encoded.push(KeyPrefix::TxLockScript as u8);
                    encoded.extend_from_slice(&script.as_slice()[SCRIPT_SERIALIZE_OFFSET..]);
                    encoded.extend_from_slice(&block_number.to_be_bytes());
                    encoded.extend_from_slice(&tx_index.to_be_bytes());
                    encoded.extend_from_slice(&io_index.to_be_bytes());
                    match io_type {
                        IOType::Input => encoded.push(0),
                        IOType::Output => encoded.push(1),
                    }
                }
                Key::TxTypeScript(script, block_number, tx_index, io_index, io_type) => {
                    encoded.push(KeyPrefix::TxTypeScript as u8);
                    encoded.extend_from_slice(&script.as_slice()[SCRIPT_SERIALIZE_OFFSET..]);
                    encoded.extend_from_slice(&block_number.to_be_bytes());
                    encoded.extend_from_slice(&tx_index.to_be_bytes());
                    encoded.extend_from_slice(&io_index.to_be_bytes());
                    match io_type {
                        IOType::Input => encoded.push(0),
                        IOType::Output => encoded.push(1),
                    }
                }
                Key::TxHash(tx_hash) => {
                    encoded.push(KeyPrefix::TxHash as u8);
                    encoded.extend_from_slice(tx_hash.as_slice());
                }
                Key::Header(block_number, block_hash) => {
                    encoded.push(KeyPrefix::Header as u8);
                    encoded.extend_from_slice(&block_number.to_be_bytes());
                    encoded.extend_from_slice(block_hash.as_slice());
                }
            }
            encoded
        }
    }
    impl<'a> Into<Vec<u8>> for Value<'a> {
        fn into(self) -> Vec<u8> {
            let mut encoded = Vec::new();
            match self {
                Value::Cell(block_number, tx_index, output, output_data) => {
                    encoded.extend_from_slice(&block_number.to_le_bytes());
                    encoded.extend_from_slice(&tx_index.to_le_bytes());
                    encoded.extend_from_slice(output.as_slice());
                    encoded.extend_from_slice(output_data.as_slice());
                }
                Value::TxHash(tx_hash) => {
                    encoded.extend_from_slice(tx_hash.as_slice());
                }
                Value::TransactionInputs(out_points) => {
                    out_points
                        .iter()
                        .for_each(|out_point| encoded.extend_from_slice(out_point.as_slice()));
                }
                Value::Transactions(txs) => {
                    txs.iter().for_each(|(tx_hash, outputs_len)| {
                        encoded.extend_from_slice(tx_hash.as_slice());
                        encoded.extend_from_slice(&(outputs_len).to_le_bytes());
                    });
                }
            }
            encoded
        }
    }
    impl<'a> Value<'a> {
        pub fn parse_cell_value(slice: &[u8]) -> (BlockNumber, TxIndex, CellOutput, Bytes) {
            let block_number = BlockNumber::from_le_bytes(
                slice[0..8].try_into().expect("stored cell block_number"),
            );
            let tx_index =
                TxIndex::from_le_bytes(slice[8..12].try_into().expect("stored cell tx_index"));
            let output_size =
                u32::from_le_bytes(slice[12..16].try_into().expect("stored cell output_size"))
                    as usize;
            let output =
                CellOutput::from_slice(&slice[12..12 + output_size]).expect("stored cell output");
            let output_data =
                Bytes::from_slice(&slice[12 + output_size..]).expect("stored cell output_data");
            (block_number, tx_index, output, output_data)
        }
        pub fn parse_transactions_value(slice: &[u8]) -> Vec<(Byte32, u32)> {
            slice
                .chunks_exact(36)
                .map(|s| {
                    let tx_hash =
                        Byte32::from_slice(&s[0..32]).expect("stored block value: tx_hash");
                    let outputs_len = u32::from_le_bytes(
                        s[32..].try_into().expect("stored block value: outputs_len"),
                    );
                    (tx_hash, outputs_len)
                })
                .collect()
        }
    }
    pub struct DetailedLiveCell {
        pub block_number: BlockNumber,
        pub block_hash: Byte32,
        pub tx_index: TxIndex,
        pub cell_output: CellOutput,
        pub cell_data: Bytes,
    }
    pub struct Indexer<S> {
        store: S,
        keep_num: u64,
        prune_interval: u64,
    }
    impl<S> Indexer<S> {
        pub fn new(store: S, keep_num: u64, prune_interval: u64) -> Self {
            Self {
                store,
                keep_num,
                prune_interval,
            }
        }
    }
    impl<S> Indexer<S>
    where
        S: Store,
    {
        pub fn append(&self, block: &BlockView) -> Result<(), Error> {
            let mut batch = self.store.batch()?;
            batch.put_kv(
                Key::Header(block.number(), &block.hash()),
                Value::Transactions(
                    block
                        .transactions()
                        .iter()
                        .map(|tx| (tx.hash(), tx.outputs().len() as u32))
                        .collect(),
                ),
            )?;
            let block_number = block.number();
            let transactions = block.transactions();
            for (tx_index, tx) in transactions.iter().enumerate() {
                let tx_index = tx_index as u32;
                let tx_hash = tx.hash();
                if tx_index > 0 {
                    for (input_index, input) in tx.inputs().into_iter().enumerate() {
                        let input_index = input_index as u32;
                        let out_point = input.previous_output();
                        let key_vec = Key::OutPoint(&out_point).into_vec();
                        let stored_live_cell = self
                            .store
                            .get(&key_vec)?
                            .or_else(|| {
                                transactions
                                    .iter()
                                    .enumerate()
                                    .find(|(_i, tx)| tx.hash() == out_point.tx_hash())
                                    .map(|(i, tx)| {
                                        Value::Cell(
                                            block_number,
                                            i as u32,
                                            &tx.outputs()
                                                .get(out_point.index().unpack())
                                                .expect("index should match"),
                                            &tx.outputs_data()
                                                .get(out_point.index().unpack())
                                                .expect("index should match"),
                                        )
                                        .into()
                                    })
                            })
                            .expect("stored live cell or consume output in same block");
                        let (
                            generated_by_block_number,
                            generated_by_tx_index,
                            output,
                            _output_data,
                        ) = Value::parse_cell_value(&stored_live_cell);
                        batch.delete(
                            Key::CellLockScript(
                                &output.lock(),
                                generated_by_block_number,
                                generated_by_tx_index,
                                out_point.index().unpack(),
                            )
                            .into_vec(),
                        )?;
                        batch.put_kv(
                            Key::TxLockScript(
                                &output.lock(),
                                block_number,
                                tx_index,
                                input_index,
                                IOType::Input,
                            ),
                            Value::TxHash(&tx_hash),
                        )?;
                        if let Some(script) = output.type_().to_opt() {
                            batch.delete(
                                Key::CellTypeScript(
                                    &script,
                                    generated_by_block_number,
                                    generated_by_tx_index,
                                    out_point.index().unpack(),
                                )
                                .into_vec(),
                            )?;
                            batch.put_kv(
                                Key::TxTypeScript(
                                    &script,
                                    block_number,
                                    tx_index,
                                    input_index,
                                    IOType::Input,
                                ),
                                Value::TxHash(&tx_hash),
                            )?;
                        };
                        batch.delete(key_vec)?;
                        batch.put_kv(
                            Key::ConsumedOutPoint(block_number, &out_point),
                            stored_live_cell,
                        )?;
                    }
                }
                for (output_index, output) in tx.outputs().into_iter().enumerate() {
                    let output_data = tx
                        .outputs_data()
                        .get(output_index)
                        .expect("outputs_data len should equals outputs len");
                    let output_index = output_index as u32;
                    let out_point = OutPoint::new(tx.hash(), output_index);
                    batch.put_kv(
                        Key::CellLockScript(&output.lock(), block_number, tx_index, output_index),
                        Value::TxHash(&tx_hash),
                    )?;
                    batch.put_kv(
                        Key::TxLockScript(
                            &output.lock(),
                            block_number,
                            tx_index,
                            output_index,
                            IOType::Output,
                        ),
                        Value::TxHash(&tx_hash),
                    )?;
                    if let Some(script) = output.type_().to_opt() {
                        batch.put_kv(
                            Key::CellTypeScript(&script, block_number, tx_index, output_index),
                            Value::TxHash(&tx_hash),
                        )?;
                        batch.put_kv(
                            Key::TxTypeScript(
                                &script,
                                block_number,
                                tx_index,
                                output_index,
                                IOType::Output,
                            ),
                            Value::TxHash(&tx_hash),
                        )?;
                    }
                    batch.put_kv(
                        Key::OutPoint(&out_point),
                        Value::Cell(block_number, tx_index, &output, &output_data),
                    )?;
                }
                batch.put_kv(
                    Key::TxHash(&tx_hash),
                    Value::TransactionInputs(
                        tx.inputs()
                            .into_iter()
                            .map(|input| input.previous_output())
                            .collect(),
                    ),
                )?;
            }
            batch.commit()?;
            if block_number % self.prune_interval == 0 {
                self.prune()?;
            }
            Ok(())
        }
        pub fn rollback(&self) -> Result<(), Error> {
            if let Some((block_number, block_hash)) = self.tip()? {
                let mut batch = self.store.batch()?;
                let txs = Value::parse_transactions_value(
                    &self
                        .store
                        .get(Key::Header(block_number, &block_hash).into_vec())?
                        .expect("stored block"),
                );
                for (tx_index, (tx_hash, outputs_len)) in txs.into_iter().enumerate().rev() {
                    let tx_index = tx_index as u32;
                    for output_index in 0..outputs_len {
                        let out_point = OutPoint::new(tx_hash.clone(), output_index);
                        let out_point_key = Key::OutPoint(&out_point).into_vec();
                        let (
                            _generated_by_block_number,
                            _generated_by_tx_index,
                            output,
                            _output_data,
                        ) = if let Some(stored_live_cell) = self.store.get(&out_point_key)? {
                            Value::parse_cell_value(&stored_live_cell)
                        } else {
                            let consumed_cell = self
                                .store
                                .get(Key::ConsumedOutPoint(block_number, &out_point).into_vec())?
                                .expect("stored live cell or consume output in same block");
                            Value::parse_cell_value(&consumed_cell)
                        };
                        batch.delete(
                            Key::CellLockScript(
                                &output.lock(),
                                block_number,
                                tx_index,
                                output_index,
                            )
                            .into_vec(),
                        )?;
                        batch.delete(
                            Key::TxLockScript(
                                &output.lock(),
                                block_number,
                                tx_index,
                                output_index,
                                IOType::Output,
                            )
                            .into_vec(),
                        )?;
                        if let Some(script) = output.type_().to_opt() {
                            batch.delete(
                                Key::CellTypeScript(&script, block_number, tx_index, output_index)
                                    .into_vec(),
                            )?;
                            batch.delete(
                                Key::TxTypeScript(
                                    &script,
                                    block_number,
                                    tx_index,
                                    output_index,
                                    IOType::Output,
                                )
                                .into_vec(),
                            )?;
                        };
                        batch.delete(out_point_key)?;
                    }
                    let transaction_key = Key::TxHash(&tx_hash).into_vec();
                    if tx_index > 0 {
                        for (input_index, out_point) in self
                            .store
                            .get(&transaction_key)?
                            .expect("stored transaction inputs")
                            .chunks_exact(OutPoint::TOTAL_SIZE)
                            .map(|slice| {
                                OutPoint::from_slice(slice)
                                    .expect("stored transaction inputs out_point slice")
                            })
                            .enumerate()
                        {
                            let input_index = input_index as u32;
                            let consumed_out_point_key =
                                Key::ConsumedOutPoint(block_number, &out_point).into_vec();
                            let stored_consumed_cell = self
                                .store
                                .get(consumed_out_point_key)?
                                .expect("stored consumed cells value");
                            let (
                                generated_by_block_number,
                                generated_by_tx_index,
                                output,
                                _output_data,
                            ) = Value::parse_cell_value(&stored_consumed_cell);
                            batch.put_kv(
                                Key::CellLockScript(
                                    &output.lock(),
                                    generated_by_block_number,
                                    generated_by_tx_index,
                                    out_point.index().unpack(),
                                ),
                                Value::TxHash(&tx_hash),
                            )?;
                            batch.delete(
                                Key::TxLockScript(
                                    &output.lock(),
                                    block_number,
                                    tx_index,
                                    input_index,
                                    IOType::Input,
                                )
                                .into_vec(),
                            )?;
                            if let Some(script) = output.type_().to_opt() {
                                batch.put_kv(
                                    Key::CellTypeScript(
                                        &script,
                                        generated_by_block_number,
                                        generated_by_tx_index,
                                        out_point.index().unpack(),
                                    ),
                                    Value::TxHash(&tx_hash),
                                )?;
                                batch.delete(
                                    Key::TxTypeScript(
                                        &script,
                                        block_number,
                                        tx_index,
                                        input_index,
                                        IOType::Input,
                                    )
                                    .into_vec(),
                                )?;
                            }
                            batch.put_kv(Key::OutPoint(&out_point), stored_consumed_cell)?;
                        }
                    }
                    batch.delete(transaction_key)?;
                }
                batch.delete(Key::Header(block_number, &block_hash).into_vec())?;
                batch.commit()?;
            }
            Ok(())
        }
        pub fn tip(&self) -> Result<Option<(BlockNumber, Byte32)>, Error> {
            let mut iter = self
                .store
                .iter(&[KeyPrefix::Header as u8 + 1], IteratorDirection::Reverse)?;
            Ok(iter.next().map(|(key, _)| {
                (
                    BlockNumber::from_be_bytes(key[1..9].try_into().expect("stored block key")),
                    Byte32::from_slice(&key[9..]).expect("stored block key"),
                )
            }))
        }
        pub fn prune(&self) -> Result<(), Error> {
            let (tip_number, _tip_hash) = self.tip()?.expect("stored tip");
            if tip_number > self.keep_num {
                let prune_to_block = tip_number - self.keep_num;
                let mut batch = self.store.batch()?;
                let key_prefix_consumed_out_point =
                    <[_]>::into_vec(box [KeyPrefix::ConsumedOutPoint as u8]);
                let iter = self
                    .store
                    .iter(&key_prefix_consumed_out_point, IteratorDirection::Forward)?;
                for (_block_number, key) in iter
                    .map(|(key, _value)| {
                        (
                            BlockNumber::from_be_bytes(
                                key[1..9].try_into().expect("stored block_number"),
                            ),
                            key,
                        )
                    })
                    .take_while(|(block_number, _key)| prune_to_block.gt(block_number))
                {
                    batch.delete(key)?;
                }
                let mut key_prefix_header = <[_]>::into_vec(box [KeyPrefix::Header as u8]);
                key_prefix_header.extend_from_slice(&prune_to_block.to_be_bytes());
                let iter = self
                    .store
                    .iter(&key_prefix_header, IteratorDirection::Reverse)?
                    .take_while(|(key, _value)| key.starts_with(&[KeyPrefix::Header as u8]));
                for txs in iter.map(|(_key, value)| Value::parse_transactions_value(&value)) {
                    let (first_tx_hash, _) = txs.get(0).expect("none empty block");
                    if self.store.exists(Key::TxHash(first_tx_hash).into_vec())? {
                        for (tx_hash, _outputs_len) in txs {
                            batch.delete(Key::TxHash(&tx_hash).into_vec())?;
                        }
                    } else {
                        break;
                    }
                }
                batch.commit()?;
            }
            Ok(())
        }
        pub fn get_live_cells_by_lock_script(
            &self,
            lock_script: &Script,
        ) -> Result<Vec<OutPoint>, Error> {
            self.get_live_cells_by_script(lock_script, KeyPrefix::CellLockScript)
        }
        pub fn get_live_cells_by_type_script(
            &self,
            type_script: &Script,
        ) -> Result<Vec<OutPoint>, Error> {
            self.get_live_cells_by_script(type_script, KeyPrefix::CellTypeScript)
        }
        fn get_live_cells_by_script(
            &self,
            script: &Script,
            prefix: KeyPrefix,
        ) -> Result<Vec<OutPoint>, Error> {
            let mut start_key = <[_]>::into_vec(box [prefix as u8]);
            start_key.extend_from_slice(&script.as_slice()[SCRIPT_SERIALIZE_OFFSET..]);
            let iter = self.store.iter(&start_key, IteratorDirection::Forward)?;
            Ok(iter
                .take_while(|(key, _)| key.starts_with(&start_key))
                .map(|(key, value)| {
                    let tx_hash = Byte32::from_slice(&value).expect("stored tx hash");
                    let index = OutputIndex::from_be_bytes(
                        key[key.len() - 4..].try_into().expect("stored index"),
                    );
                    OutPoint::new(tx_hash, index)
                })
                .collect())
        }
        pub fn get_transactions_by_lock_script(
            &self,
            lock_script: &Script,
        ) -> Result<Vec<Byte32>, Error> {
            self.get_transactions_by_script(lock_script, KeyPrefix::TxLockScript)
        }
        pub fn get_transactions_by_type_script(
            &self,
            type_script: &Script,
        ) -> Result<Vec<Byte32>, Error> {
            self.get_transactions_by_script(type_script, KeyPrefix::TxTypeScript)
        }
        fn get_transactions_by_script(
            &self,
            script: &Script,
            prefix: KeyPrefix,
        ) -> Result<Vec<Byte32>, Error> {
            let mut start_key = <[_]>::into_vec(box [prefix as u8]);
            start_key.extend_from_slice(&script.as_slice()[SCRIPT_SERIALIZE_OFFSET..]);
            let iter = self.store.iter(&start_key, IteratorDirection::Forward)?;
            Ok(iter
                .take_while(|(key, _)| key.starts_with(&start_key))
                .map(|(_key, value)| Byte32::from_slice(&value).expect("stored tx hash"))
                .collect())
        }
        #[doc = " Given an OutPoint representing a live cell, returns the following components"]
        #[doc = " related to the live cell:"]
        #[doc = " * CellOutput"]
        #[doc = " * Cell data"]
        #[doc = " * Block hash in which the cell is created"]
        pub fn get_detailed_live_cell(
            &self,
            out_point: &OutPoint,
        ) -> Result<Option<DetailedLiveCell>, Error> {
            let key_vec = Key::OutPoint(&out_point).into_vec();
            let (block_number, tx_index, cell_output, cell_data) = match self.store.get(&key_vec)? {
                Some(stored_cell) => Value::parse_cell_value(&stored_cell),
                None => return Ok(None),
            };
            let mut header_start_key = <[_]>::into_vec(box [KeyPrefix::Header as u8]);
            header_start_key.extend_from_slice(&block_number.to_be_bytes());
            let mut iter = self
                .store
                .iter(&header_start_key, IteratorDirection::Forward)?;
            let block_hash = match iter.next() {
                Some((key, _)) => {
                    if key.starts_with(&header_start_key) {
                        let start = std::mem::size_of::<BlockNumber>() + 1;
                        Byte32::from_slice(&key[start..start + 32]).expect("stored key header hash")
                    } else {
                        return Ok(None);
                    }
                }
                None => return Ok(None),
            };
            Ok(Some(DetailedLiveCell {
                block_number,
                block_hash,
                tx_index,
                cell_output,
                cell_data,
            }))
        }
        pub fn report(&self) -> Result<(), Error> {
            let iter = self.store.iter(&[], IteratorDirection::Forward)?;
            let mut statistics: HashMap<u8, (usize, usize, usize)> = HashMap::new();
            for (key, value) in iter {
                let s = statistics.entry(*key.first().unwrap()).or_default();
                s.0 += 1;
                s.1 += key.len();
                s.2 += value.len();
            }
            {
                ::std::io::_print(::core::fmt::Arguments::new_v1(
                    &["", "\n"],
                    &match (&statistics,) {
                        (arg0,) => [::core::fmt::ArgumentV1::new(arg0, ::core::fmt::Debug::fmt)],
                    },
                ));
            };
            Ok(())
        }
    }
    pub enum Error {
        StoreError(String),
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl ::core::fmt::Debug for Error {
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            match (&*self,) {
                (&Error::StoreError(ref __self_0),) => {
                    let mut debug_trait_builder = f.debug_tuple("StoreError");
                    let _ = debug_trait_builder.field(&&(*__self_0));
                    debug_trait_builder.finish()
                }
            }
        }
    }
    impl From<StoreError> for Error {
        fn from(e: StoreError) -> Error {
            match e {
                StoreError::DBError(s) => Error::StoreError(s),
            }
        }
    }
}
pub mod service {
    use crate::indexer::{Indexer, Key, KeyPrefix, Value};
    use crate::store::{IteratorDirection, Store};
    use ckb_jsonrpc_types::{
        BlockNumber, BlockView, CellOutput, JsonBytes, OutPoint, Script, Uint32,
    };
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
                .cors(DomainsValidation::AllowOnly(<[_]>::into_vec(box [
                    AccessControlAllowOrigin::Null,
                    AccessControlAllowOrigin::Any,
                ])))
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
                            {
                                let lvl = ::log::Level::Info;
                                if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                                    ::log::__private_api_log(
                                        ::core::fmt::Arguments::new_v1(
                                            &["append ", ", "],
                                            &match (&block.number(), &block.hash()) {
                                                (arg0, arg1) => [
                                                    ::core::fmt::ArgumentV1::new(
                                                        arg0,
                                                        ::core::fmt::Display::fmt,
                                                    ),
                                                    ::core::fmt::ArgumentV1::new(
                                                        arg1,
                                                        ::core::fmt::Display::fmt,
                                                    ),
                                                ],
                                            },
                                        ),
                                        lvl,
                                        &(
                                            "ckb_indexer::service",
                                            "ckb_indexer::service",
                                            "src/service.rs",
                                            68u32,
                                        ),
                                    );
                                }
                            };
                            indexer.append(&block).unwrap();
                        } else {
                            {
                                let lvl = ::log::Level::Info;
                                if lvl <= ::log::STATIC_MAX_LEVEL && lvl <= ::log::max_level() {
                                    ::log::__private_api_log(
                                        ::core::fmt::Arguments::new_v1(
                                            &["rollback ", ", "],
                                            &match (&tip_number, &tip_hash) {
                                                (arg0, arg1) => [
                                                    ::core::fmt::ArgumentV1::new(
                                                        arg0,
                                                        ::core::fmt::Display::fmt,
                                                    ),
                                                    ::core::fmt::ArgumentV1::new(
                                                        arg1,
                                                        ::core::fmt::Display::fmt,
                                                    ),
                                                ],
                                            },
                                        ),
                                        lvl,
                                        &(
                                            "ckb_indexer::service",
                                            "ckb_indexer::service",
                                            "src/service.rs",
                                            71u32,
                                        ),
                                    );
                                }
                            };
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
    mod rpc_impl_CkbRpc {
        use super::*;
        use jsonrpc_core as _jsonrpc_core;
        #[doc = r" The generated client module."]
        pub mod gen_client {
            use super::*;
            use _jsonrpc_core::futures::prelude::*;
            use _jsonrpc_core::futures::sync::{mpsc, oneshot};
            use _jsonrpc_core::serde_json::{self, Value};
            use _jsonrpc_core::{
                Call, Error, ErrorCode, Id, MethodCall, Params, Request, Response, Version,
            };
            use _jsonrpc_core_client::{
                RpcChannel, RpcError, RpcFuture, TypedClient, TypedSubscriptionStream,
            };
            use jsonrpc_core_client as _jsonrpc_core_client;
            #[doc = r" The Client."]
            pub struct Client {
                inner: TypedClient,
            }
            #[automatically_derived]
            #[allow(unused_qualifications)]
            impl ::core::clone::Clone for Client {
                #[inline]
                fn clone(&self) -> Client {
                    match *self {
                        Client {
                            inner: ref __self_0_0,
                        } => Client {
                            inner: ::core::clone::Clone::clone(&(*__self_0_0)),
                        },
                    }
                }
            }
            impl Client {
                #[doc = r" Creates a new `Client`."]
                pub fn new(sender: RpcChannel) -> Self {
                    Client {
                        inner: sender.into(),
                    }
                }
                pub fn get_block_by_number(
                    &self,
                    _number: BlockNumber,
                ) -> impl Future<Item = Option<BlockView>, Error = RpcError> {
                    let args_tuple = (_number,);
                    self.inner.call_method(
                        "get_block_by_number",
                        "Option < BlockView >",
                        args_tuple,
                    )
                }
            }
            impl From<RpcChannel> for Client {
                fn from(channel: RpcChannel) -> Self {
                    Client::new(channel.into())
                }
            }
        }
    }
    pub use self::rpc_impl_CkbRpc::gen_client;
    mod rpc_impl_IndexerRpc {
        use super::*;
        use jsonrpc_core as _jsonrpc_core;
        #[doc = r" The generated server module."]
        pub mod gen_server {
            use self::_jsonrpc_core::futures as _futures;
            use super::*;
            pub trait IndexerRpc: Sized + Send + Sync + 'static {
                fn get_cells(
                    &self,
                    search_key: SearchKey,
                    order: Order,
                    limit: Uint32,
                    after: Option<JsonBytes>,
                ) -> Result<Pagination<Cell>>;
                fn get_transactions(
                    &self,
                    search_key: SearchKey,
                    order: Order,
                    limit: Uint32,
                    after: Option<JsonBytes>,
                ) -> Result<Pagination<Tx>>;
                #[doc = r" Create an `IoDelegate`, wiring rpc calls to the trait methods."]
                fn to_delegate<M: _jsonrpc_core::Metadata>(
                    self,
                ) -> _jsonrpc_core::IoDelegate<Self, M> {
                    let mut del = _jsonrpc_core::IoDelegate::new(self.into());
                    del.add_method("get_cells", move |base, params| {
                        let method = &(Self::get_cells
                            as fn(
                                &Self,
                                SearchKey,
                                Order,
                                Uint32,
                                Option<JsonBytes>,
                            ) -> Result<Pagination<Cell>>);
                        let passed_args_num = match params {
                            _jsonrpc_core::Params::Array(ref v) => Ok(v.len()),
                            _jsonrpc_core::Params::None => Ok(0),
                            _ => Err(_jsonrpc_core::Error::invalid_params(
                                "`params` should be an array",
                            )),
                        };
                        let params =
                            passed_args_num.and_then(|passed_args_num| match passed_args_num {
                                _ if passed_args_num < 3usize => {
                                    Err(_jsonrpc_core::Error::invalid_params({
                                        let res =
                                            ::alloc::fmt::format(::core::fmt::Arguments::new_v1(
                                                &["`params` should have at least ", " argument(s)"],
                                                &match (&3usize,) {
                                                    (arg0,) => [::core::fmt::ArgumentV1::new(
                                                        arg0,
                                                        ::core::fmt::Display::fmt,
                                                    )],
                                                },
                                            ));
                                        res
                                    }))
                                }
                                3usize => params
                                    .parse::<(SearchKey, Order, Uint32)>()
                                    .map(|(a, b, c)| (a, b, c, None))
                                    .map_err(Into::into),
                                4usize => params
                                    .parse::<(SearchKey, Order, Uint32, Option<JsonBytes>)>()
                                    .map(|(a, b, c, d)| (a, b, c, d))
                                    .map_err(Into::into),
                                _ => Err(_jsonrpc_core::Error::invalid_params_with_details(
                                    {
                                        let res =
                                            ::alloc::fmt::format(::core::fmt::Arguments::new_v1(
                                                &["Expected from ", " to ", " parameters."],
                                                &match (&3usize, &4usize) {
                                                    (arg0, arg1) => [
                                                        ::core::fmt::ArgumentV1::new(
                                                            arg0,
                                                            ::core::fmt::Display::fmt,
                                                        ),
                                                        ::core::fmt::ArgumentV1::new(
                                                            arg1,
                                                            ::core::fmt::Display::fmt,
                                                        ),
                                                    ],
                                                },
                                            ));
                                        res
                                    },
                                    {
                                        let res =
                                            ::alloc::fmt::format(::core::fmt::Arguments::new_v1(
                                                &["Got: "],
                                                &match (&passed_args_num,) {
                                                    (arg0,) => [::core::fmt::ArgumentV1::new(
                                                        arg0,
                                                        ::core::fmt::Display::fmt,
                                                    )],
                                                },
                                            ));
                                        res
                                    },
                                )),
                            });
                        match params {
                            Ok((a, b, c, d)) => {
                                use self::_futures::{Future, IntoFuture};
                                let fut = (method)(base, a, b, c, d)
                                    .into_future()
                                    .map(|value| {
                                        _jsonrpc_core::to_value(value)
                                            .expect("Expected always-serializable type; qed")
                                    })
                                    .map_err(Into::into as fn(_) -> _jsonrpc_core::Error);
                                _futures::future::Either::A(fut)
                            }
                            Err(e) => _futures::future::Either::B(_futures::failed(e)),
                        }
                    });
                    del.add_method("get_transactions", move |base, params| {
                        let method = &(Self::get_transactions
                            as fn(
                                &Self,
                                SearchKey,
                                Order,
                                Uint32,
                                Option<JsonBytes>,
                            ) -> Result<Pagination<Tx>>);
                        let passed_args_num = match params {
                            _jsonrpc_core::Params::Array(ref v) => Ok(v.len()),
                            _jsonrpc_core::Params::None => Ok(0),
                            _ => Err(_jsonrpc_core::Error::invalid_params(
                                "`params` should be an array",
                            )),
                        };
                        let params =
                            passed_args_num.and_then(|passed_args_num| match passed_args_num {
                                _ if passed_args_num < 3usize => {
                                    Err(_jsonrpc_core::Error::invalid_params({
                                        let res =
                                            ::alloc::fmt::format(::core::fmt::Arguments::new_v1(
                                                &["`params` should have at least ", " argument(s)"],
                                                &match (&3usize,) {
                                                    (arg0,) => [::core::fmt::ArgumentV1::new(
                                                        arg0,
                                                        ::core::fmt::Display::fmt,
                                                    )],
                                                },
                                            ));
                                        res
                                    }))
                                }
                                3usize => params
                                    .parse::<(SearchKey, Order, Uint32)>()
                                    .map(|(a, b, c)| (a, b, c, None))
                                    .map_err(Into::into),
                                4usize => params
                                    .parse::<(SearchKey, Order, Uint32, Option<JsonBytes>)>()
                                    .map(|(a, b, c, d)| (a, b, c, d))
                                    .map_err(Into::into),
                                _ => Err(_jsonrpc_core::Error::invalid_params_with_details(
                                    {
                                        let res =
                                            ::alloc::fmt::format(::core::fmt::Arguments::new_v1(
                                                &["Expected from ", " to ", " parameters."],
                                                &match (&3usize, &4usize) {
                                                    (arg0, arg1) => [
                                                        ::core::fmt::ArgumentV1::new(
                                                            arg0,
                                                            ::core::fmt::Display::fmt,
                                                        ),
                                                        ::core::fmt::ArgumentV1::new(
                                                            arg1,
                                                            ::core::fmt::Display::fmt,
                                                        ),
                                                    ],
                                                },
                                            ));
                                        res
                                    },
                                    {
                                        let res =
                                            ::alloc::fmt::format(::core::fmt::Arguments::new_v1(
                                                &["Got: "],
                                                &match (&passed_args_num,) {
                                                    (arg0,) => [::core::fmt::ArgumentV1::new(
                                                        arg0,
                                                        ::core::fmt::Display::fmt,
                                                    )],
                                                },
                                            ));
                                        res
                                    },
                                )),
                            });
                        match params {
                            Ok((a, b, c, d)) => {
                                use self::_futures::{Future, IntoFuture};
                                let fut = (method)(base, a, b, c, d)
                                    .into_future()
                                    .map(|value| {
                                        _jsonrpc_core::to_value(value)
                                            .expect("Expected always-serializable type; qed")
                                    })
                                    .map_err(Into::into as fn(_) -> _jsonrpc_core::Error);
                                _futures::future::Either::A(fut)
                            }
                            Err(e) => _futures::future::Either::B(_futures::failed(e)),
                        }
                    });
                    del
                }
            }
        }
    }
    pub use self::rpc_impl_IndexerRpc::gen_server::IndexerRpc;
    pub struct SearchKey {
        script: Script,
        script_type: ScriptType,
        args_len: Option<Uint32>,
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _IMPL_DESERIALIZE_FOR_SearchKey: () = {
        #[allow(rust_2018_idioms, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl<'de> _serde::Deserialize<'de> for SearchKey {
            fn deserialize<__D>(__deserializer: __D) -> _serde::export::Result<Self, __D::Error>
            where
                __D: _serde::Deserializer<'de>,
            {
                #[allow(non_camel_case_types)]
                enum __Field {
                    __field0,
                    __field1,
                    __field2,
                    __ignore,
                }
                struct __FieldVisitor;
                impl<'de> _serde::de::Visitor<'de> for __FieldVisitor {
                    type Value = __Field;
                    fn expecting(
                        &self,
                        __formatter: &mut _serde::export::Formatter,
                    ) -> _serde::export::fmt::Result {
                        _serde::export::Formatter::write_str(__formatter, "field identifier")
                    }
                    fn visit_u64<__E>(
                        self,
                        __value: u64,
                    ) -> _serde::export::Result<Self::Value, __E>
                    where
                        __E: _serde::de::Error,
                    {
                        match __value {
                            0u64 => _serde::export::Ok(__Field::__field0),
                            1u64 => _serde::export::Ok(__Field::__field1),
                            2u64 => _serde::export::Ok(__Field::__field2),
                            _ => _serde::export::Err(_serde::de::Error::invalid_value(
                                _serde::de::Unexpected::Unsigned(__value),
                                &"field index 0 <= i < 3",
                            )),
                        }
                    }
                    fn visit_str<__E>(
                        self,
                        __value: &str,
                    ) -> _serde::export::Result<Self::Value, __E>
                    where
                        __E: _serde::de::Error,
                    {
                        match __value {
                            "script" => _serde::export::Ok(__Field::__field0),
                            "script_type" => _serde::export::Ok(__Field::__field1),
                            "args_len" => _serde::export::Ok(__Field::__field2),
                            _ => _serde::export::Ok(__Field::__ignore),
                        }
                    }
                    fn visit_bytes<__E>(
                        self,
                        __value: &[u8],
                    ) -> _serde::export::Result<Self::Value, __E>
                    where
                        __E: _serde::de::Error,
                    {
                        match __value {
                            b"script" => _serde::export::Ok(__Field::__field0),
                            b"script_type" => _serde::export::Ok(__Field::__field1),
                            b"args_len" => _serde::export::Ok(__Field::__field2),
                            _ => _serde::export::Ok(__Field::__ignore),
                        }
                    }
                }
                impl<'de> _serde::Deserialize<'de> for __Field {
                    #[inline]
                    fn deserialize<__D>(
                        __deserializer: __D,
                    ) -> _serde::export::Result<Self, __D::Error>
                    where
                        __D: _serde::Deserializer<'de>,
                    {
                        _serde::Deserializer::deserialize_identifier(__deserializer, __FieldVisitor)
                    }
                }
                struct __Visitor<'de> {
                    marker: _serde::export::PhantomData<SearchKey>,
                    lifetime: _serde::export::PhantomData<&'de ()>,
                }
                impl<'de> _serde::de::Visitor<'de> for __Visitor<'de> {
                    type Value = SearchKey;
                    fn expecting(
                        &self,
                        __formatter: &mut _serde::export::Formatter,
                    ) -> _serde::export::fmt::Result {
                        _serde::export::Formatter::write_str(__formatter, "struct SearchKey")
                    }
                    #[inline]
                    fn visit_seq<__A>(
                        self,
                        mut __seq: __A,
                    ) -> _serde::export::Result<Self::Value, __A::Error>
                    where
                        __A: _serde::de::SeqAccess<'de>,
                    {
                        let __field0 =
                            match match _serde::de::SeqAccess::next_element::<Script>(&mut __seq) {
                                _serde::export::Ok(__val) => __val,
                                _serde::export::Err(__err) => {
                                    return _serde::export::Err(__err);
                                }
                            } {
                                _serde::export::Some(__value) => __value,
                                _serde::export::None => {
                                    return _serde::export::Err(_serde::de::Error::invalid_length(
                                        0usize,
                                        &"struct SearchKey with 3 elements",
                                    ));
                                }
                            };
                        let __field1 = match match _serde::de::SeqAccess::next_element::<ScriptType>(
                            &mut __seq,
                        ) {
                            _serde::export::Ok(__val) => __val,
                            _serde::export::Err(__err) => {
                                return _serde::export::Err(__err);
                            }
                        } {
                            _serde::export::Some(__value) => __value,
                            _serde::export::None => {
                                return _serde::export::Err(_serde::de::Error::invalid_length(
                                    1usize,
                                    &"struct SearchKey with 3 elements",
                                ));
                            }
                        };
                        let __field2 = match match _serde::de::SeqAccess::next_element::<
                            Option<Uint32>,
                        >(&mut __seq)
                        {
                            _serde::export::Ok(__val) => __val,
                            _serde::export::Err(__err) => {
                                return _serde::export::Err(__err);
                            }
                        } {
                            _serde::export::Some(__value) => __value,
                            _serde::export::None => {
                                return _serde::export::Err(_serde::de::Error::invalid_length(
                                    2usize,
                                    &"struct SearchKey with 3 elements",
                                ));
                            }
                        };
                        _serde::export::Ok(SearchKey {
                            script: __field0,
                            script_type: __field1,
                            args_len: __field2,
                        })
                    }
                    #[inline]
                    fn visit_map<__A>(
                        self,
                        mut __map: __A,
                    ) -> _serde::export::Result<Self::Value, __A::Error>
                    where
                        __A: _serde::de::MapAccess<'de>,
                    {
                        let mut __field0: _serde::export::Option<Script> = _serde::export::None;
                        let mut __field1: _serde::export::Option<ScriptType> = _serde::export::None;
                        let mut __field2: _serde::export::Option<Option<Uint32>> =
                            _serde::export::None;
                        while let _serde::export::Some(__key) =
                            match _serde::de::MapAccess::next_key::<__Field>(&mut __map) {
                                _serde::export::Ok(__val) => __val,
                                _serde::export::Err(__err) => {
                                    return _serde::export::Err(__err);
                                }
                            }
                        {
                            match __key {
                                __Field::__field0 => {
                                    if _serde::export::Option::is_some(&__field0) {
                                        return _serde::export::Err(
                                            <__A::Error as _serde::de::Error>::duplicate_field(
                                                "script",
                                            ),
                                        );
                                    }
                                    __field0 = _serde::export::Some(
                                        match _serde::de::MapAccess::next_value::<Script>(
                                            &mut __map,
                                        ) {
                                            _serde::export::Ok(__val) => __val,
                                            _serde::export::Err(__err) => {
                                                return _serde::export::Err(__err);
                                            }
                                        },
                                    );
                                }
                                __Field::__field1 => {
                                    if _serde::export::Option::is_some(&__field1) {
                                        return _serde::export::Err(
                                            <__A::Error as _serde::de::Error>::duplicate_field(
                                                "script_type",
                                            ),
                                        );
                                    }
                                    __field1 = _serde::export::Some(
                                        match _serde::de::MapAccess::next_value::<ScriptType>(
                                            &mut __map,
                                        ) {
                                            _serde::export::Ok(__val) => __val,
                                            _serde::export::Err(__err) => {
                                                return _serde::export::Err(__err);
                                            }
                                        },
                                    );
                                }
                                __Field::__field2 => {
                                    if _serde::export::Option::is_some(&__field2) {
                                        return _serde::export::Err(
                                            <__A::Error as _serde::de::Error>::duplicate_field(
                                                "args_len",
                                            ),
                                        );
                                    }
                                    __field2 = _serde::export::Some(
                                        match _serde::de::MapAccess::next_value::<Option<Uint32>>(
                                            &mut __map,
                                        ) {
                                            _serde::export::Ok(__val) => __val,
                                            _serde::export::Err(__err) => {
                                                return _serde::export::Err(__err);
                                            }
                                        },
                                    );
                                }
                                _ => {
                                    let _ = match _serde::de::MapAccess::next_value::<
                                        _serde::de::IgnoredAny,
                                    >(&mut __map)
                                    {
                                        _serde::export::Ok(__val) => __val,
                                        _serde::export::Err(__err) => {
                                            return _serde::export::Err(__err);
                                        }
                                    };
                                }
                            }
                        }
                        let __field0 = match __field0 {
                            _serde::export::Some(__field0) => __field0,
                            _serde::export::None => {
                                match _serde::private::de::missing_field("script") {
                                    _serde::export::Ok(__val) => __val,
                                    _serde::export::Err(__err) => {
                                        return _serde::export::Err(__err);
                                    }
                                }
                            }
                        };
                        let __field1 = match __field1 {
                            _serde::export::Some(__field1) => __field1,
                            _serde::export::None => {
                                match _serde::private::de::missing_field("script_type") {
                                    _serde::export::Ok(__val) => __val,
                                    _serde::export::Err(__err) => {
                                        return _serde::export::Err(__err);
                                    }
                                }
                            }
                        };
                        let __field2 = match __field2 {
                            _serde::export::Some(__field2) => __field2,
                            _serde::export::None => {
                                match _serde::private::de::missing_field("args_len") {
                                    _serde::export::Ok(__val) => __val,
                                    _serde::export::Err(__err) => {
                                        return _serde::export::Err(__err);
                                    }
                                }
                            }
                        };
                        _serde::export::Ok(SearchKey {
                            script: __field0,
                            script_type: __field1,
                            args_len: __field2,
                        })
                    }
                }
                const FIELDS: &'static [&'static str] = &["script", "script_type", "args_len"];
                _serde::Deserializer::deserialize_struct(
                    __deserializer,
                    "SearchKey",
                    FIELDS,
                    __Visitor {
                        marker: _serde::export::PhantomData::<SearchKey>,
                        lifetime: _serde::export::PhantomData,
                    },
                )
            }
        }
    };
    #[serde(rename_all = "snake_case")]
    pub enum ScriptType {
        Lock,
        Type,
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _IMPL_DESERIALIZE_FOR_ScriptType: () = {
        #[allow(rust_2018_idioms, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl<'de> _serde::Deserialize<'de> for ScriptType {
            fn deserialize<__D>(__deserializer: __D) -> _serde::export::Result<Self, __D::Error>
            where
                __D: _serde::Deserializer<'de>,
            {
                #[allow(non_camel_case_types)]
                enum __Field {
                    __field0,
                    __field1,
                }
                struct __FieldVisitor;
                impl<'de> _serde::de::Visitor<'de> for __FieldVisitor {
                    type Value = __Field;
                    fn expecting(
                        &self,
                        __formatter: &mut _serde::export::Formatter,
                    ) -> _serde::export::fmt::Result {
                        _serde::export::Formatter::write_str(__formatter, "variant identifier")
                    }
                    fn visit_u64<__E>(
                        self,
                        __value: u64,
                    ) -> _serde::export::Result<Self::Value, __E>
                    where
                        __E: _serde::de::Error,
                    {
                        match __value {
                            0u64 => _serde::export::Ok(__Field::__field0),
                            1u64 => _serde::export::Ok(__Field::__field1),
                            _ => _serde::export::Err(_serde::de::Error::invalid_value(
                                _serde::de::Unexpected::Unsigned(__value),
                                &"variant index 0 <= i < 2",
                            )),
                        }
                    }
                    fn visit_str<__E>(
                        self,
                        __value: &str,
                    ) -> _serde::export::Result<Self::Value, __E>
                    where
                        __E: _serde::de::Error,
                    {
                        match __value {
                            "lock" => _serde::export::Ok(__Field::__field0),
                            "type" => _serde::export::Ok(__Field::__field1),
                            _ => _serde::export::Err(_serde::de::Error::unknown_variant(
                                __value, VARIANTS,
                            )),
                        }
                    }
                    fn visit_bytes<__E>(
                        self,
                        __value: &[u8],
                    ) -> _serde::export::Result<Self::Value, __E>
                    where
                        __E: _serde::de::Error,
                    {
                        match __value {
                            b"lock" => _serde::export::Ok(__Field::__field0),
                            b"type" => _serde::export::Ok(__Field::__field1),
                            _ => {
                                let __value = &_serde::export::from_utf8_lossy(__value);
                                _serde::export::Err(_serde::de::Error::unknown_variant(
                                    __value, VARIANTS,
                                ))
                            }
                        }
                    }
                }
                impl<'de> _serde::Deserialize<'de> for __Field {
                    #[inline]
                    fn deserialize<__D>(
                        __deserializer: __D,
                    ) -> _serde::export::Result<Self, __D::Error>
                    where
                        __D: _serde::Deserializer<'de>,
                    {
                        _serde::Deserializer::deserialize_identifier(__deserializer, __FieldVisitor)
                    }
                }
                struct __Visitor<'de> {
                    marker: _serde::export::PhantomData<ScriptType>,
                    lifetime: _serde::export::PhantomData<&'de ()>,
                }
                impl<'de> _serde::de::Visitor<'de> for __Visitor<'de> {
                    type Value = ScriptType;
                    fn expecting(
                        &self,
                        __formatter: &mut _serde::export::Formatter,
                    ) -> _serde::export::fmt::Result {
                        _serde::export::Formatter::write_str(__formatter, "enum ScriptType")
                    }
                    fn visit_enum<__A>(
                        self,
                        __data: __A,
                    ) -> _serde::export::Result<Self::Value, __A::Error>
                    where
                        __A: _serde::de::EnumAccess<'de>,
                    {
                        match match _serde::de::EnumAccess::variant(__data) {
                            _serde::export::Ok(__val) => __val,
                            _serde::export::Err(__err) => {
                                return _serde::export::Err(__err);
                            }
                        } {
                            (__Field::__field0, __variant) => {
                                match _serde::de::VariantAccess::unit_variant(__variant) {
                                    _serde::export::Ok(__val) => __val,
                                    _serde::export::Err(__err) => {
                                        return _serde::export::Err(__err);
                                    }
                                };
                                _serde::export::Ok(ScriptType::Lock)
                            }
                            (__Field::__field1, __variant) => {
                                match _serde::de::VariantAccess::unit_variant(__variant) {
                                    _serde::export::Ok(__val) => __val,
                                    _serde::export::Err(__err) => {
                                        return _serde::export::Err(__err);
                                    }
                                };
                                _serde::export::Ok(ScriptType::Type)
                            }
                        }
                    }
                }
                const VARIANTS: &'static [&'static str] = &["lock", "type"];
                _serde::Deserializer::deserialize_enum(
                    __deserializer,
                    "ScriptType",
                    VARIANTS,
                    __Visitor {
                        marker: _serde::export::PhantomData::<ScriptType>,
                        lifetime: _serde::export::PhantomData,
                    },
                )
            }
        }
    };
    #[serde(rename_all = "snake_case")]
    pub enum Order {
        Desc,
        Asc,
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _IMPL_DESERIALIZE_FOR_Order: () = {
        #[allow(rust_2018_idioms, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl<'de> _serde::Deserialize<'de> for Order {
            fn deserialize<__D>(__deserializer: __D) -> _serde::export::Result<Self, __D::Error>
            where
                __D: _serde::Deserializer<'de>,
            {
                #[allow(non_camel_case_types)]
                enum __Field {
                    __field0,
                    __field1,
                }
                struct __FieldVisitor;
                impl<'de> _serde::de::Visitor<'de> for __FieldVisitor {
                    type Value = __Field;
                    fn expecting(
                        &self,
                        __formatter: &mut _serde::export::Formatter,
                    ) -> _serde::export::fmt::Result {
                        _serde::export::Formatter::write_str(__formatter, "variant identifier")
                    }
                    fn visit_u64<__E>(
                        self,
                        __value: u64,
                    ) -> _serde::export::Result<Self::Value, __E>
                    where
                        __E: _serde::de::Error,
                    {
                        match __value {
                            0u64 => _serde::export::Ok(__Field::__field0),
                            1u64 => _serde::export::Ok(__Field::__field1),
                            _ => _serde::export::Err(_serde::de::Error::invalid_value(
                                _serde::de::Unexpected::Unsigned(__value),
                                &"variant index 0 <= i < 2",
                            )),
                        }
                    }
                    fn visit_str<__E>(
                        self,
                        __value: &str,
                    ) -> _serde::export::Result<Self::Value, __E>
                    where
                        __E: _serde::de::Error,
                    {
                        match __value {
                            "desc" => _serde::export::Ok(__Field::__field0),
                            "asc" => _serde::export::Ok(__Field::__field1),
                            _ => _serde::export::Err(_serde::de::Error::unknown_variant(
                                __value, VARIANTS,
                            )),
                        }
                    }
                    fn visit_bytes<__E>(
                        self,
                        __value: &[u8],
                    ) -> _serde::export::Result<Self::Value, __E>
                    where
                        __E: _serde::de::Error,
                    {
                        match __value {
                            b"desc" => _serde::export::Ok(__Field::__field0),
                            b"asc" => _serde::export::Ok(__Field::__field1),
                            _ => {
                                let __value = &_serde::export::from_utf8_lossy(__value);
                                _serde::export::Err(_serde::de::Error::unknown_variant(
                                    __value, VARIANTS,
                                ))
                            }
                        }
                    }
                }
                impl<'de> _serde::Deserialize<'de> for __Field {
                    #[inline]
                    fn deserialize<__D>(
                        __deserializer: __D,
                    ) -> _serde::export::Result<Self, __D::Error>
                    where
                        __D: _serde::Deserializer<'de>,
                    {
                        _serde::Deserializer::deserialize_identifier(__deserializer, __FieldVisitor)
                    }
                }
                struct __Visitor<'de> {
                    marker: _serde::export::PhantomData<Order>,
                    lifetime: _serde::export::PhantomData<&'de ()>,
                }
                impl<'de> _serde::de::Visitor<'de> for __Visitor<'de> {
                    type Value = Order;
                    fn expecting(
                        &self,
                        __formatter: &mut _serde::export::Formatter,
                    ) -> _serde::export::fmt::Result {
                        _serde::export::Formatter::write_str(__formatter, "enum Order")
                    }
                    fn visit_enum<__A>(
                        self,
                        __data: __A,
                    ) -> _serde::export::Result<Self::Value, __A::Error>
                    where
                        __A: _serde::de::EnumAccess<'de>,
                    {
                        match match _serde::de::EnumAccess::variant(__data) {
                            _serde::export::Ok(__val) => __val,
                            _serde::export::Err(__err) => {
                                return _serde::export::Err(__err);
                            }
                        } {
                            (__Field::__field0, __variant) => {
                                match _serde::de::VariantAccess::unit_variant(__variant) {
                                    _serde::export::Ok(__val) => __val,
                                    _serde::export::Err(__err) => {
                                        return _serde::export::Err(__err);
                                    }
                                };
                                _serde::export::Ok(Order::Desc)
                            }
                            (__Field::__field1, __variant) => {
                                match _serde::de::VariantAccess::unit_variant(__variant) {
                                    _serde::export::Ok(__val) => __val,
                                    _serde::export::Err(__err) => {
                                        return _serde::export::Err(__err);
                                    }
                                };
                                _serde::export::Ok(Order::Asc)
                            }
                        }
                    }
                }
                const VARIANTS: &'static [&'static str] = &["desc", "asc"];
                _serde::Deserializer::deserialize_enum(
                    __deserializer,
                    "Order",
                    VARIANTS,
                    __Visitor {
                        marker: _serde::export::PhantomData::<Order>,
                        lifetime: _serde::export::PhantomData,
                    },
                )
            }
        }
    };
    pub struct Cell {
        output: CellOutput,
        output_data: JsonBytes,
        out_point: OutPoint,
        block_number: BlockNumber,
        tx_index: Uint32,
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _IMPL_SERIALIZE_FOR_Cell: () = {
        #[allow(rust_2018_idioms, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for Cell {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::export::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = match _serde::Serializer::serialize_struct(
                    __serializer,
                    "Cell",
                    false as usize + 1 + 1 + 1 + 1 + 1,
                ) {
                    _serde::export::Ok(__val) => __val,
                    _serde::export::Err(__err) => {
                        return _serde::export::Err(__err);
                    }
                };
                match _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "output",
                    &self.output,
                ) {
                    _serde::export::Ok(__val) => __val,
                    _serde::export::Err(__err) => {
                        return _serde::export::Err(__err);
                    }
                };
                match _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "output_data",
                    &self.output_data,
                ) {
                    _serde::export::Ok(__val) => __val,
                    _serde::export::Err(__err) => {
                        return _serde::export::Err(__err);
                    }
                };
                match _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "out_point",
                    &self.out_point,
                ) {
                    _serde::export::Ok(__val) => __val,
                    _serde::export::Err(__err) => {
                        return _serde::export::Err(__err);
                    }
                };
                match _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "block_number",
                    &self.block_number,
                ) {
                    _serde::export::Ok(__val) => __val,
                    _serde::export::Err(__err) => {
                        return _serde::export::Err(__err);
                    }
                };
                match _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "tx_index",
                    &self.tx_index,
                ) {
                    _serde::export::Ok(__val) => __val,
                    _serde::export::Err(__err) => {
                        return _serde::export::Err(__err);
                    }
                };
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    pub struct Tx {
        tx_hash: H256,
        block_number: BlockNumber,
        tx_index: Uint32,
        io_index: Uint32,
        io_type: IOType,
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _IMPL_SERIALIZE_FOR_Tx: () = {
        #[allow(rust_2018_idioms, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for Tx {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::export::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = match _serde::Serializer::serialize_struct(
                    __serializer,
                    "Tx",
                    false as usize + 1 + 1 + 1 + 1 + 1,
                ) {
                    _serde::export::Ok(__val) => __val,
                    _serde::export::Err(__err) => {
                        return _serde::export::Err(__err);
                    }
                };
                match _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "tx_hash",
                    &self.tx_hash,
                ) {
                    _serde::export::Ok(__val) => __val,
                    _serde::export::Err(__err) => {
                        return _serde::export::Err(__err);
                    }
                };
                match _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "block_number",
                    &self.block_number,
                ) {
                    _serde::export::Ok(__val) => __val,
                    _serde::export::Err(__err) => {
                        return _serde::export::Err(__err);
                    }
                };
                match _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "tx_index",
                    &self.tx_index,
                ) {
                    _serde::export::Ok(__val) => __val,
                    _serde::export::Err(__err) => {
                        return _serde::export::Err(__err);
                    }
                };
                match _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "io_index",
                    &self.io_index,
                ) {
                    _serde::export::Ok(__val) => __val,
                    _serde::export::Err(__err) => {
                        return _serde::export::Err(__err);
                    }
                };
                match _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "io_type",
                    &self.io_type,
                ) {
                    _serde::export::Ok(__val) => __val,
                    _serde::export::Err(__err) => {
                        return _serde::export::Err(__err);
                    }
                };
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
    #[serde(rename_all = "snake_case")]
    pub enum IOType {
        Input,
        Output,
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _IMPL_SERIALIZE_FOR_IOType: () = {
        #[allow(rust_2018_idioms, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl _serde::Serialize for IOType {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::export::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                match *self {
                    IOType::Input => _serde::Serializer::serialize_unit_variant(
                        __serializer,
                        "IOType",
                        0u32,
                        "input",
                    ),
                    IOType::Output => _serde::Serializer::serialize_unit_variant(
                        __serializer,
                        "IOType",
                        1u32,
                        "output",
                    ),
                }
            }
        }
    };
    pub struct Pagination<T> {
        objects: Vec<T>,
        last_cursor: JsonBytes,
    }
    #[doc(hidden)]
    #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
    const _IMPL_SERIALIZE_FOR_Pagination: () = {
        #[allow(rust_2018_idioms, clippy::useless_attribute)]
        extern crate serde as _serde;
        #[automatically_derived]
        impl<T> _serde::Serialize for Pagination<T>
        where
            T: _serde::Serialize,
        {
            fn serialize<__S>(
                &self,
                __serializer: __S,
            ) -> _serde::export::Result<__S::Ok, __S::Error>
            where
                __S: _serde::Serializer,
            {
                let mut __serde_state = match _serde::Serializer::serialize_struct(
                    __serializer,
                    "Pagination",
                    false as usize + 1 + 1,
                ) {
                    _serde::export::Ok(__val) => __val,
                    _serde::export::Err(__err) => {
                        return _serde::export::Err(__err);
                    }
                };
                match _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "objects",
                    &self.objects,
                ) {
                    _serde::export::Ok(__val) => __val,
                    _serde::export::Err(__err) => {
                        return _serde::export::Err(__err);
                    }
                };
                match _serde::ser::SerializeStruct::serialize_field(
                    &mut __serde_state,
                    "last_cursor",
                    &self.last_cursor,
                ) {
                    _serde::export::Ok(__val) => __val,
                    _serde::export::Err(__err) => {
                        return _serde::export::Err(__err);
                    }
                };
                _serde::ser::SerializeStruct::end(__serde_state)
            }
        }
    };
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
                ScriptType::Lock => <[_]>::into_vec(box [KeyPrefix::CellLockScript as u8]),
                ScriptType::Type => <[_]>::into_vec(box [KeyPrefix::CellTypeScript as u8]),
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
                        || {
                            (
                                [
                                    prefix.clone(),
                                    ::alloc::vec::from_elem(0xff, remain_args_len + 16),
                                ]
                                .concat(),
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
                ScriptType::Lock => <[_]>::into_vec(box [KeyPrefix::TxLockScript as u8]),
                ScriptType::Type => <[_]>::into_vec(box [KeyPrefix::TxTypeScript as u8]),
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
                        || {
                            (
                                [
                                    prefix.clone(),
                                    ::alloc::vec::from_elem(0xff, remain_args_len + 17),
                                ]
                                .concat(),
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
}
pub mod store {
    mod rocksdb {
        use super::{Batch, Error, IteratorDirection, IteratorItem, Store};
        use rocksdb::{Direction, IteratorMode, WriteBatch, DB};
        use std::sync::Arc;
        pub struct RocksdbStore {
            db: Arc<DB>,
        }
        #[automatically_derived]
        #[allow(unused_qualifications)]
        impl ::core::clone::Clone for RocksdbStore {
            #[inline]
            fn clone(&self) -> RocksdbStore {
                match *self {
                    RocksdbStore { db: ref __self_0_0 } => RocksdbStore {
                        db: ::core::clone::Clone::clone(&(*__self_0_0)),
                    },
                }
            }
        }
        impl Store for RocksdbStore {
            type Batch = RocksdbBatch;
            fn new(path: &str) -> Self {
                let db = Arc::new(DB::open_default(path).expect("Failed to open rocksdb"));
                Self { db }
            }
            fn get<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<Vec<u8>>, Error> {
                self.db
                    .get(key.as_ref())
                    .map(|v| v.map(|vi| vi.to_vec()))
                    .map_err(Into::into)
            }
            fn exists<K: AsRef<[u8]>>(&self, key: K) -> Result<bool, Error> {
                self.db
                    .get(key.as_ref())
                    .map(|v| v.is_some())
                    .map_err(Into::into)
            }
            fn iter<K: AsRef<[u8]>>(
                &self,
                from_key: K,
                mode: IteratorDirection,
            ) -> Result<Box<dyn Iterator<Item = IteratorItem>>, Error> {
                let mode = IteratorMode::From(
                    from_key.as_ref(),
                    match mode {
                        IteratorDirection::Forward => Direction::Forward,
                        IteratorDirection::Reverse => Direction::Reverse,
                    },
                );
                Ok(Box::new(self.db.iterator(mode)) as Box<_>)
            }
            fn batch(&self) -> Result<Self::Batch, Error> {
                Ok(Self::Batch {
                    db: Arc::clone(&self.db),
                    wb: WriteBatch::default(),
                })
            }
        }
        pub struct RocksdbBatch {
            db: Arc<DB>,
            wb: WriteBatch,
        }
        impl Batch for RocksdbBatch {
            fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(
                &mut self,
                key: K,
                value: V,
            ) -> Result<(), Error> {
                self.wb.put(key, value)?;
                Ok(())
            }
            fn delete<K: AsRef<[u8]>>(&mut self, key: K) -> Result<(), Error> {
                self.wb.delete(key.as_ref())?;
                Ok(())
            }
            fn commit(self) -> Result<(), Error> {
                self.db.write(self.wb)?;
                Ok(())
            }
        }
        impl From<rocksdb::Error> for Error {
            fn from(e: rocksdb::Error) -> Error {
                Error::DBError(e.to_string())
            }
        }
    }
    pub use self::rocksdb::RocksdbStore;
    pub enum Error {
        DBError(String),
    }
    #[automatically_derived]
    #[allow(unused_qualifications)]
    impl ::core::fmt::Debug for Error {
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            match (&*self,) {
                (&Error::DBError(ref __self_0),) => {
                    let mut debug_trait_builder = f.debug_tuple("DBError");
                    let _ = debug_trait_builder.field(&&(*__self_0));
                    debug_trait_builder.finish()
                }
            }
        }
    }
    pub type IteratorItem = (Box<[u8]>, Box<[u8]>);
    pub enum IteratorDirection {
        Forward,
        Reverse,
    }
    pub trait Store {
        type Batch: Batch;
        fn new(path: &str) -> Self;
        fn get<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<Vec<u8>>, Error>;
        fn exists<K: AsRef<[u8]>>(&self, key: K) -> Result<bool, Error>;
        fn iter<K: AsRef<[u8]>>(
            &self,
            from_key: K,
            direction: IteratorDirection,
        ) -> Result<Box<dyn Iterator<Item = IteratorItem>>, Error>;
        fn batch(&self) -> Result<Self::Batch, Error>;
    }
    pub trait Batch {
        fn put_kv<K: Into<Vec<u8>>, V: Into<Vec<u8>>>(
            &mut self,
            key: K,
            value: V,
        ) -> Result<(), Error> {
            self.put(&Into::<Vec<u8>>::into(key), &Into::<Vec<u8>>::into(value))
        }
        fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(&mut self, key: K, value: V) -> Result<(), Error>;
        fn delete<K: AsRef<[u8]>>(&mut self, key: K) -> Result<(), Error>;
        fn commit(self) -> Result<(), Error>;
    }
}
