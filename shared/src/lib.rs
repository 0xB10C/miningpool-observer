#[macro_use]
extern crate diesel;

pub mod config;
pub mod db_pool;
pub mod model;
pub mod schema;
pub mod tags;

// Minimal and incorrect HTTP server answering on all requests with
// the prometheus metrics.
pub mod prometheus_metric_server;

// Re-export bitcoincore_rpc to only need to upgrade it in one place.
pub extern crate bitcoincore_rpc;
