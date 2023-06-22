use std::sync::{Arc, Mutex};
use warp::Filter;

pub fn status_route(
    shared_state: Arc<Mutex<Option<String>>>,
) -> warp::filters::BoxedFilter<(impl warp::Reply,)> {
    warp::path("is_ready")
        .and(warp::get())
        .and_then(move || is_ready(shared_state.clone()))
        .boxed()
}

pub async fn is_ready(
    shared_state: Arc<Mutex<Option<String>>>,
) -> Result<impl warp::Reply, warp::Rejection> {
    let state = shared_state.lock().unwrap();
    match &*state {
        Some(state) => {
            println!("State: {}", state);
            Ok(warp::reply::json(state))
        }
        None => Ok(warp::reply::json(
            &"Service is running, but no state available yet",
        )),
    }
}
