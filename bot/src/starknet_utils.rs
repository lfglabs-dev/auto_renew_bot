use crate::{config::Config, models::TxResult};
use starknet::{
    core::types::{
        MaybePendingTransactionReceipt, PendingTransactionReceipt, TransactionExecutionStatus,
        TransactionReceipt,
    },
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider},
};
use url::Url;

pub fn create_jsonrpc_client(conf: &Config) -> JsonRpcClient<HttpTransport> {
    JsonRpcClient::new(HttpTransport::new(Url::parse(&conf.rpc.rpc_url).unwrap()))
}

pub async fn check_pending_transactions(conf: &Config, tx_results: &mut Vec<TxResult>) {
    let client = create_jsonrpc_client(conf);
    for tx_result in tx_results.iter_mut() {
        if tx_result.reverted.is_none() {
            match client.get_transaction_receipt(tx_result.tx_hash).await {
                Ok(receipt) => match receipt {
                    MaybePendingTransactionReceipt::PendingReceipt(pending_receipt) => {
                        if let PendingTransactionReceipt::Invoke(invocation) = pending_receipt {
                            match invocation.execution_result.status() {
                                TransactionExecutionStatus::Succeeded => {
                                    tx_result.reverted = Some(false);
                                }
                                TransactionExecutionStatus::Reverted => {
                                    tx_result.reverted = Some(true);
                                    tx_result.revert_reason = invocation
                                        .execution_result
                                        .revert_reason()
                                        .map(|s| s.to_owned());
                                }
                            }
                        }
                    }
                    MaybePendingTransactionReceipt::Receipt(receipt) => {
                        if let TransactionReceipt::Invoke(invocation) = receipt {
                            match invocation.execution_result.status() {
                                TransactionExecutionStatus::Succeeded => {
                                    tx_result.reverted = Some(false);
                                }
                                TransactionExecutionStatus::Reverted => {
                                    tx_result.reverted = Some(true);
                                    tx_result.revert_reason = invocation
                                        .execution_result
                                        .revert_reason()
                                        .map(|s| s.to_owned());
                                }
                            }
                        }
                    }
                },
                Err(e) => {
                    eprintln!(
                        "Error checking status for tx_hash {}: {}",
                        tx_result.tx_hash, e
                    );
                }
            }
        }
    }
}
