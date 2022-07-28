use ckb_indexer::{pool::Pool, service::Service};
use ckb_jsonrpc_types::{PoolTransactionEntry, PoolTransactionReject};
use ckb_types::packed::Transaction;
use clap::{crate_version, App, Arg};
use futures::{SinkExt, StreamExt, TryStreamExt};
use jsonrpc_core_client::{
    transports::{duplex::duplex, http},
    TypedClient,
};
use jsonrpc_server_utils::{
    codecs::StreamCodec,
    tokio::{self, net::TcpStream},
    tokio_util::codec::Decoder,
};
use log::{debug, error};
use std::process::exit;
use std::sync::{Arc, RwLock};

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
                .help("CKB rpc service uri, supports http and tcp, for example: `http://127.0.0.1:8114` or `tcp://127.0.0.1:18114`")
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
        .arg(
            Arg::with_name("index_tx_pool")
                .long("index-tx-pool")
                .help("Whether to index the pending txs in the ckb tx-pool")
                .required(false)
        )
        .arg(
            Arg::with_name("block_filter")
                .long("block-filter")
                .help("A custom block filter rule to filter out blocks that are not interesting to the indexer")
                .required(false)
                .takes_value(true),
        )
        .arg(
            Arg::with_name("cell_filter")
                .long("cell-filter")
                .help("A custom cell filter rule to filter out cells that are not interesting to the indexer")
                .required(false)
                .takes_value(true),
        )
        .get_matches();

    let index_tx_pool = matches.is_present("index_tx_pool");
    let block_filter = matches.value_of("block_filter");
    let cell_filter = matches.value_of("cell_filter");
    if index_tx_pool && (block_filter.is_some() || cell_filter.is_some()) {
        error!("cannot use customized block or cell filter rule when `index-tx-pool` is enabled");
        exit(1);
    }

    let pool = Arc::new(RwLock::new(Pool::default()));
    let service = Service::new(
        matches.value_of("store_path").expect("required arg"),
        if index_tx_pool {
            Some(pool.clone())
        } else {
            None
        },
        matches.value_of("listen_uri").unwrap_or("127.0.0.1:8116"),
        std::time::Duration::from_secs(2),
        crate_version!().to_string(),
    );

    let rpc_server = service.start();
    let mut uri = matches
        .value_of("ckb_uri")
        .unwrap_or("http://127.0.0.1:8114")
        .to_owned();

    if uri.starts_with("tcp://") {
        let uri = uri.split_off(6);
        let codec = StreamCodec::stream_incoming();
        let tcp_stream = TcpStream::connect(&uri)
            .await
            .unwrap_or_else(|_| panic!("Failed to connect to {:?}", uri));
        let (sink, stream) = codec.framed(tcp_stream).split();
        let sink = sink.sink_map_err(|e| error!("tcp sink error: {:?}", e));
        let stream = stream.map_err(|e| error!("tcp stream error: {:?}", e));

        let (duplex, rpc_channel) = duplex(
            Box::pin(sink),
            Box::pin(
                stream
                    .take_while(|x| futures::future::ready(x.is_ok()))
                    .map(|x| x.expect("stream is closed on errors")),
            ),
        );
        tokio::spawn(duplex);

        if index_tx_pool {
            // subscribe `new_transaction` topic
            {
                let typed_client: TypedClient = rpc_channel.clone().into();
                let pool = pool.clone();
                let fut = async move {
                    match typed_client.subscribe::<_, String>(
                        "subscribe",
                        ["new_transaction"],
                        "subscribe",
                        "unsubscribe",
                        "String",
                    ) {
                        Ok(mut stream) => loop {
                            if let Some(subscription) = stream.next().await {
                                match subscription {
                                    Ok(json_string) => {
                                        debug!(
                                            "Rpc subscription notified a new transaction: {}",
                                            json_string
                                        );
                                        if let Ok(pool_tx_entry) =
                                            serde_json::from_str::<PoolTransactionEntry>(
                                                &json_string,
                                            )
                                        {
                                            let tx: Transaction =
                                                pool_tx_entry.transaction.inner.into();
                                            pool.write()
                                                .expect("acquire lock")
                                                .new_transaction(&tx.into_view());
                                        } else {
                                            error!("Failed to parse json_string: {}", json_string);
                                        }
                                    }
                                    Err(err) => {
                                        error!("Rpc subscription error {:?}", err);
                                    }
                                }
                            }
                        },
                        Err(err) => {
                            error!("subscribe error {:?}", err);
                        }
                    }
                };
                tokio::spawn(fut);
            }

            // subscribe `rejected_transaction` topic
            {
                let typed_client: TypedClient = rpc_channel.clone().into();
                let pool = pool.clone();
                let fut = async move {
                    match typed_client.subscribe::<_, String>(
                        "subscribe",
                        ["rejected_transaction"],
                        "subscribe",
                        "unsubscribe",
                        "String",
                    ) {
                        Ok(mut stream) => loop {
                            if let Some(subscription) = stream.next().await {
                                match subscription {
                                    Ok(json_string) => {
                                        debug!(
                                            "Rpc subscription notified a rejected transaction: {}",
                                            json_string
                                        );
                                        if let Ok((pool_tx_entry, _)) = serde_json::from_str::<(
                                            PoolTransactionEntry,
                                            PoolTransactionReject,
                                        )>(
                                            &json_string
                                        ) {
                                            let tx: Transaction =
                                                pool_tx_entry.transaction.inner.into();
                                            pool.write()
                                                .expect("acquire lock")
                                                .transaction_rejected(&tx.into_view());
                                        } else {
                                            error!("Failed to parse json_string: {}", json_string);
                                        }
                                    }
                                    Err(err) => {
                                        error!("Rpc subscription error {:?}", err);
                                    }
                                }
                            }
                        },
                        Err(err) => {
                            error!("subscribe error {:?}", err);
                        }
                    }
                };
                tokio::spawn(fut);
            }
        }

        service.poll(rpc_channel.into(), None, None).await;
    } else if index_tx_pool {
        error!("indexing the pending txs in the ckb tx-pool is only supported when connecting to ckb rpc service with tcp protocol")
    } else {
        let client = http::connect(&uri)
            .await
            .unwrap_or_else(|_| panic!("Failed to connect to {:?}", uri));
        service.poll(client, block_filter, cell_filter).await;
    }

    rpc_server.close();
}
