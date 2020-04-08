mod rocksdb;

pub use self::rocksdb::RocksdbStore;

#[derive(Debug)]
pub enum Error {
    DBError(String),
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
