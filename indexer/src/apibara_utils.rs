use crate::models::AppState;
use apibara_core::node::v1alpha2::Cursor;
use mongodb::{
    bson::{doc, Bson, Document},
    options::{FindOneAndUpdateOptions, ReturnDocument},
};
use std::sync::Arc;

fn _add_chain_information(doc: &mut Document, order_key: u64) {
    doc.insert(
        "_chain",
        doc! {
            "valid_from": order_key as i64,
            "valid_to": Bson::Null
        },
    );
}

pub async fn insert_one(
    state: &Arc<AppState>,
    collection: &str,
    mut doc: Document,
    order_key: u64,
) -> Result<mongodb::results::InsertOneResult, Box<dyn std::error::Error>> {
    _add_chain_information(&mut doc, order_key);
    let coll = state.db.collection::<Document>(collection);
    let result = coll.insert_one(doc, None).await?;
    Ok(result)
}

fn _add_current_block_to_filter(filter: &mut Document) {
    filter.insert("_chain.valid_to", Bson::Null);
}

pub async fn find_one_and_update(
    state: &Arc<AppState>,
    collection: &str,
    mut filter: Document,
    update: Document,
    order_key: u64,
) -> Result<Option<Document>, Box<dyn std::error::Error>> {
    _add_current_block_to_filter(&mut filter);

    let update_old = doc! {
        "$set": {
            "_chain.valid_to": order_key as i64
        }
    };

    let options = FindOneAndUpdateOptions::builder()
        .return_document(ReturnDocument::After)
        .build();

    let coll = state.db.collection::<Document>(collection);
    let existing = coll
        .find_one_and_update(filter.clone(), update_old, options)
        .await?;

    // Step 2. To simulate an update, first insert then call update on it.
    if let Some(mut existing_doc) = existing.clone() {
        existing_doc.remove("_id");
        existing_doc.remove("_chain");
        coll.insert_one(existing_doc, None).await?;
        coll.update_one(filter, update, None).await?;
    }

    Ok(existing)
}

pub async fn invalidate(state: &Arc<AppState>, cursor: Option<Cursor>) {
    match cursor {
        Some(cursor) => {
            let cursor_key = cursor.order_key as i64;
            // collection.delete_many({"_chain.valid_from": {"$gt": cursor.order_key }}
            for collection_name in state.db.list_collection_names(None).await.unwrap() {
                if collection_name.starts_with("_") {
                    continue;
                }
                let collection = state.db.collection::<bson::Document>(&collection_name);
                // remove items inserted after block_number
                collection
                    .delete_many(doc! { "_chain.valid_from": { "$gt": cursor_key } }, None)
                    .await
                    .unwrap();
                // rollback items updated after block_number
                collection
                    .update_many(
                        doc! { "_chain.valid_to": { "$gt": cursor_key } },
                        doc! { "$set": { "_chain.valid_to": Bson::Null } },
                        None,
                    )
                    .await
                    .unwrap();
            }
        }
        None => {
            panic!("Invalid cursor");
        }
    }
}
