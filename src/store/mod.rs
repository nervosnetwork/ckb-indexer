mod rocksdb;

pub use self::rocksdb::RocksdbStore;

use anyhow::Result;
use derive_more::Display;

#[derive(Debug, Display)]
pub enum Error {
    #[display(fmt = "DB Error {:?}", _0)]
    DBError(String),
}

impl std::error::Error for Error {}

pub type IteratorItem = (Box<[u8]>, Box<[u8]>);

pub enum IteratorDirection {
    Forward,
    Reverse,
}

pub trait Store {
    type Batch: Batch;
    fn new(path: &str) -> Self;
    fn get<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<Vec<u8>>>;
    fn exists<K: AsRef<[u8]>>(&self, key: K) -> Result<bool>;
    fn iter<K: AsRef<[u8]>>(
        &self,
        from_key: K,
        direction: IteratorDirection,
    ) -> Result<Box<dyn Iterator<Item = IteratorItem> + '_>>;
    fn batch(&self) -> Result<Self::Batch>;
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
