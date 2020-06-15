use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use surf::http_types::Error;
pub struct Client<'a> {
    pub uri: &'a str,
    pub id: Arc<Mutex<u64>>,
}

/// A JSONRPC request object
#[derive(Debug, Clone, Serialize)]
pub struct RpcRequest<'a> {
    /// The name of the RPC call
    pub method: &'a str,
    /// Parameters to the RPC call
    pub params: &'a [Value],
    /// Identifier for this Request, which should appear in the response
    pub id: Value,
    /// jsonrpc field, MUST be "2.0"
    pub jsonrpc: Option<&'a str>,
}

/// A JSONRPC response object
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RpcResponse {
    /// A result if there is one, or null
    pub result: Option<Value>,
    /// An error if there is one, or null
    pub error: Option<RpcError>,
    /// Identifier for this Request, which should match that of the request
    pub id: Value,
    /// jsonrpc field, MUST be "2.0"
    pub jsonrpc: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
/// A JSONRPC error object
pub struct RpcError {
    /// The integer identifier of the error
    pub code: i32,
    /// A string describing the error
    pub message: String,
    /// Additional data specific to the error
    pub data: Option<Value>,
}

impl<'a> Client<'a> {
    pub fn new(uri: &'a str) -> Self {
        Client {
            uri: uri,
            id: Arc::new(Mutex::new(0)),
        }
    }

    pub fn build_request(&self, method: &'a str, params: &'a [Value]) -> RpcRequest<'a> {
        let mut id = self.id.lock().unwrap();
        *id += 1;
        RpcRequest {
            method: method,
            params: params,
            id: (*id).into(),
            jsonrpc: Some("2.0"),
        }
    }

    pub async fn send_request(&self, request: RpcRequest<'a>) -> Result<RpcResponse, Error> {
        let data = serde_json::json!(request);
        let response: RpcResponse = surf::post(self.uri)
            .body_json(&data)?
            .recv_json::<RpcResponse>()
            .await?;
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_block_by_number_works() {
        crate::logger::init_log();
        async_std::task::block_on(async {
            let client = Client::new("http://127.0.0.1:8114");
            let block_number = [serde_json::json!("0x400")];
            let request = client.build_request("get_block_by_number", &block_number);
            if let Ok(response) = client.send_request(request).await {
                // let result_json =
                //     serde_json::from_value::<BlockView>(response.result.unwrap()).unwrap();
                // // let block: BlockView = result_json.into();
                log::debug!("response : {:?}", response);
            } else {
                assert!(false)
            }
        })
    }
}
