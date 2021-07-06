use ckb_indexer::service::Service;
use clap::{crate_version, App, Arg};
use jsonrpc_core_client::transports::http;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_default_env()
        .format_timestamp(Some(env_logger::fmt::TimestampPrecision::Millis))
        .init();
    let matches = App::new("ckb-indexer")
        .version(crate_version!())
        .arg(
            Arg::with_name("ckb_uri")
                .short("c")
                .help("CKB rpc http service uri, default http://127.0.0.1:8114")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("listen_uri")
                .short("l")
                .help("Indexer rpc http service listen address, default 127.0.0.1:8116")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("store_path")
                .short("s")
                .help("Sets the indexer store path to use")
                .required(true)
                .takes_value(true),
        )
        .get_matches();

    let service = Service::new(
        matches.value_of("store_path").expect("required arg"),
        matches.value_of("listen_uri").unwrap_or("127.0.0.1:8116"),
        std::time::Duration::from_secs(2),
        crate_version!().to_string(),
    );

    let rpc_server = service.start();
    let mut uri = matches
        .value_of("ckb_uri")
        .unwrap_or("http://127.0.0.1:8114")
        .to_owned();
    if !uri.starts_with("http") {
        uri = format!("http://{}", uri);
    }

    let client = http::connect(&uri)
        .await
        .expect(&format!("Failed to connect to {:?}", uri));

    service.poll(client).await;

    rpc_server.close();
}
