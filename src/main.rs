use std::error::Error as StdError;
use futures::future::Either::{Left, Right};
use futures::Stream;
use jsonrpc_core::{BoxFuture, Params};
use crate::tonlib::{AsyncClient, BlockIdExt, ClientBuilder, InternalTransactionId, MasterchainInfo, RawTransaction, ShortTxId};
use jsonrpc_http_server::jsonrpc_core::IoHandler;
use jsonrpc_http_server::tokio::runtime::Runtime;
use jsonrpc_http_server::{ServerBuilder};
use jsonrpc_derive::rpc;
use jsonrpc_core::{Result, Error};
use serde_json::{json, Value};
use serde::Deserialize;
use tokio_stream::StreamExt;
#[macro_use]
extern crate lazy_static;

mod tonlib;

lazy_static! {
    static ref TON: AsyncClient = {
        let client = Runtime::new().unwrap().block_on(async {
            ClientBuilder::from_file("./liteserver_config.json")
                .unwrap()
                // .disable_logging()
                .build()
                .await
                .unwrap()
        });

        client
    };
}

#[derive(Deserialize, Debug)]
struct LookupBlockParams {
    workchain: i64,
    shard: String,
    seqno: Option<u64>,
    lt: Option<i64>,
    unixtime: Option<u64>
}

#[derive(Deserialize, Debug)]
struct ShardsParams {
    seqno: u64
}

#[derive(Deserialize, Debug)]
struct BlockHeaderParams {
    workchain: i64,
    shard: String,
    seqno: u64,
    root_hash: Option<String>,
    file_hash: Option<String>
}

#[derive(Deserialize, Debug)]
struct BlockTransactionsParams {
    workchain: i64,
    shard: String,
    seqno: u64,
    root_hash: Option<String>,
    file_hash: Option<String>,
    after_lt: Option<i64>,
    after_hash: Option<String>,
    count: Option<u8>
}

#[derive(Deserialize, Debug)]
struct AddressParams {
    address: String
}

#[derive(Deserialize, Debug)]
struct TransactionsParams {
    address: String,
    limit: Option<u16>,
    lt: Option<String>,
    hash: Option<String>,
    to_lt: Option<String>,
    archival: Option<bool>
}

#[derive(Deserialize, Debug)]
struct SendBocParams {
    boc: String
}

type RpcResponse<T> = BoxFuture<Result<T>>;

#[rpc(server)]
pub trait Rpc {
    #[rpc(name = "getMasterchainInfo")]
    fn master_chain_info(&self) -> RpcResponse<MasterchainInfo>;

    #[rpc(name = "lookupBlock", raw_params)]
    fn lookup_block(&self, params: Params) -> RpcResponse<Value>;

    #[rpc(name = "shards", raw_params)]
    fn shards(&self, params: Params) -> RpcResponse<Value>;

    #[rpc(name = "getBlockHeader", raw_params)]
    fn get_block_header(&self, params: Params) -> RpcResponse<Value>;

    #[rpc(name = "getBlockTransactions", raw_params)]
    fn get_block_transactions(&self, params: Params) -> RpcResponse<Value>;

    #[rpc(name = "getAddressInformation", raw_params)]
    fn get_address_information(&self, params: Params) -> RpcResponse<Value>;

    #[rpc(name = "getExtendedAddressInformation", raw_params)]
    fn get_extended_address_information(&self, params: Params) -> RpcResponse<Value>;

    #[rpc(name = "getTransactions", raw_params)]
    fn get_transactions(&self, params: Params) -> RpcResponse<Value>;

    #[rpc(name = "sendBoc", raw_params)]
    fn send_boc(&self, params: Params) -> RpcResponse<Value>;
}

struct RpcImpl;

impl Rpc for RpcImpl {
    fn master_chain_info(&self) -> RpcResponse<MasterchainInfo> {
        Box::pin(async {
            jsonrpc_error(TON.get_masterchain_info().await)
        })
    }

    fn lookup_block(&self, params: Params) -> RpcResponse<Value> {
        Box::pin(async move {
            let params = params.parse::<LookupBlockParams>()?;

            let workchain = params.workchain;
            let shard = params.shard.parse::<i64>().map_err(|_| Error::invalid_params("invalid shard"))?;
            match (params.seqno, params.lt, params.unixtime) {
                (Some(seqno), None, None) if seqno > 0 => jsonrpc_error(TON.look_up_block_by_seqno(workchain, shard, seqno).await),
                (None, Some(lt), None) if lt > 0 => jsonrpc_error(TON.look_up_block_by_lt(workchain, shard, lt).await),
                (None, None, Some(_)) => Err(Error::invalid_params("unixtime is not supported")),
                _ => Err(Error::invalid_params("seqno or lt or unixtime must be provided"))
            }
        })
    }

    fn shards(&self, params: Params) -> RpcResponse<Value> {
        Box::pin(async move {
            let params = params.parse::<ShardsParams>()?;

            jsonrpc_error(TON.get_shards(params.seqno).await)
        })
    }

    fn get_block_header(&self, params: Params) -> RpcResponse<Value> {
        Box::pin(async move {
            let params = params.parse::<BlockHeaderParams>()?;
            let shard = params.shard.parse::<i64>().map_err(|_| Error::invalid_params("invalid shard"))?;


            jsonrpc_error(TON.get_block_header(
                params.workchain,
                shard,
                params.seqno
            ).await)
        })
    }

    fn get_block_transactions(&self, params: Params) -> RpcResponse<Value> {
        Box::pin(async move {
            let params = params.parse::<BlockTransactionsParams>()?;
            let shard = params.shard.parse::<i64>().map_err(|_| Error::invalid_params("invalid shard"))?;
            let count = params.count.unwrap_or(200);

            let block_json = TON
                .look_up_block_by_seqno(params.workchain, shard, params.seqno)
                .await.map_err(|_| Error::internal_error())?;

            let block = serde_json::from_value::<BlockIdExt>(block_json)
                .map_err(|_| Error::internal_error())?;

            let stream = TON.get_tx_stream(block.clone()).await;
            let tx: Vec<ShortTxId> = stream
                .map(|tx: ShortTxId| {
                    println!("{}", &tx.account);
                    ShortTxId {
                        account: format!("{}:{}", block.workchain, base64_to_hex(&tx.account).unwrap()),
                        hash: tx.hash,
                        lt: tx.lt,
                        mode: tx.mode
                    }
                })
                .collect()
                .await;


            Ok(json!({
                "@type": "blocks.transactions",
                "id": block,
                "incomplete": false,
                "req_count": count,
                "transactions": &tx
            }))
        })
    }

    fn get_address_information(&self, params: Params) -> RpcResponse<Value> {
        Box::pin(async move {
            let params = params.parse::<AddressParams>()?;

            jsonrpc_error(TON.raw_get_account_state(&params.address).await)
        })
    }

    fn get_extended_address_information(&self, params: Params) -> RpcResponse<Value> {
        Box::pin(async move {
            let params = params.parse::<AddressParams>()?;

            jsonrpc_error(TON.get_account_state(&params.address).await)
        })
    }

    fn get_transactions(&self, params: Params) -> RpcResponse<Value> {
        Box::pin(async move {
            let params = params.parse::<TransactionsParams>()?;
            let address = params.address;
            let count = params.limit.unwrap_or(10);
            let max_lt = params.to_lt.and_then(|x| x.parse::<i64>().ok());
            let lt = params.lt;
            let hash = params.hash;

            let stream = match (lt, hash) {
                (Some(lt), Some(hash)) => Left(
                    TON.get_account_tx_stream_from(address, InternalTransactionId {hash, lt})
                ),
                _ => Right(TON.get_account_tx_stream(address).await)
            };
            let stream = match max_lt {
                Some(to_lt) => Left(stream.take_while(move |tx: &RawTransaction|
                    tx.transaction_id.lt.parse::<i64>().unwrap() > to_lt
                )),
                _ => Right(stream)
            };

            let txs: Vec<RawTransaction> = stream
                .take(count as usize)
                .collect()
                .await;

            serde_json::to_value(txs).map_err(|e| {
                Error::internal_error()
            })
        })

    }

    fn send_boc(&self, params: Params) -> RpcResponse<Value> {
        Box::pin(async move {
            let params = params.parse::<SendBocParams>()?;
            let boc = base64::decode(params.boc)
                .map_err(|e| jsonrpc_core::Error::invalid_params(e.description()))?;
            let b64 = base64::encode(boc);

            jsonrpc_error(TON.send_message(&b64).await)
        })
    }
}

fn jsonrpc_error<T>(r: anyhow::Result<T>) -> Result<T> {
    r.map_err(|_| Error::internal_error())
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let block = TON.synchronize().await?;
    println!("Synchronized");

    tokio::task::spawn_blocking(|| {
        let mut io = IoHandler::new();
        io.extend_with(RpcImpl.to_delegate());

        let server = ServerBuilder::new(io)
            .start_http(&"127.0.0.1:3030".parse().unwrap())
            .unwrap();

        server.wait()
    }).await;

    Ok(())
}

fn base64_to_hex(b: &str) -> anyhow::Result<String> {
    let bytes = base64::decode(b)?;
    let hex = hex::encode(bytes);

    return Ok(hex)
}
