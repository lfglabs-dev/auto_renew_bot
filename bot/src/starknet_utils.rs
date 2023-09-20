use crate::config::Config;
use starknet::providers::{jsonrpc::HttpTransport, JsonRpcClient};
use url::Url;

pub fn create_jsonrpc_client(conf: &Config) -> JsonRpcClient<HttpTransport> {
    JsonRpcClient::new(HttpTransport::new(Url::parse(&conf.rpc.rpc_url).unwrap()))
}
