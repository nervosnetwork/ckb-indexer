use ckb_indexer::service::Service;
use futures::Future;
use hyper::rt;
use jsonrpc_core_client::transports::http;

fn main() {
    drop(env_logger::init());
    let service = Service::new("/tmp/ckb-indexer-test", std::time::Duration::from_secs(2));
    rt::run(rt::lazy(move || {
        let uri = "http://localhost:8114";

        http::connect(uri)
            .and_then(move |client| {
                service.poll(client);
                Ok(())
            })
            .map_err(|e| {
                println!("Error: {:?}", e);
            })
    }))
}
