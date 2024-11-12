#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_mut)]

use actix_web::middleware::Logger as ALogger;
use actix_web::{
    cookie::time::util::weeks_in_year, get, post, web, App, HttpResponse, HttpServer, Responder,
    ResponseError,
};
use codec::{Decode, Encode};
use ethers::utils::keccak256;
use flexi_logger::{Age, Cleanup, Criterion, Logger, Naming, WriteMode};
use http::status::{InvalidStatusCode, StatusCode};
use log::{error, info};
use smt_primitives::{
    keccak_hasher::Keccak256Hasher,
    verify::{verify as smt_verify, Proof},
};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use smt_backend_lib::req::{ReqByKey, ReqNextRoot, ReqRoot, ReqUpdate};
use smt_backend_lib::MultiSMTStore;
use sparse_merkle_tree::{traits::Value, H256};
use std::path::Path;
use std::result::Result;
use std::sync::Mutex;
use thiserror::Error as ThisError;
use tokio::signal::ctrl_c;
use smt_backend_lib::kvs::*;
use smt_backend_lib::error::Error;



#[get("/test")]
async fn test() -> impl Responder {
    let h = "hello, smt!";
    log::info!("{:?}", format!("[Test] info: {:?}", h));
    HttpResponse::Ok().body(h)
}

#[post("/update")]
async fn update(
    multi_tree: web::Data<Mutex<MultiSMTStore<SMTKey, SMTValue, Keccak256Hasher>>>,
    info: web::Json<ReqUpdate<SMTKey, SMTValue>>,
) -> Result<HttpResponse, Error> {
    let mut multi_tree = multi_tree
        .lock()
        .map_err(|e| Error::InternalError(e.to_string()))?;
    let root = multi_tree
        .update(info.prefix.as_ref(), info.key.clone(), info.value.clone())
        .map_err(|e| Error::InternalError(e.to_string()))?;
    log::info!(
        "{:#?}",
        format!("[Update] info: {:#?}, root: {:?}", info, root)
    );
    Ok(HttpResponse::Ok().json(root))
}

#[get("/get_merkle_proof")]
async fn get_merkle_proof(
    multi_tree: web::Data<Mutex<MultiSMTStore<SMTKey, SMTValue, Keccak256Hasher>>>,
    info: web::Json<ReqByKey<SMTKey>>,
) -> Result<HttpResponse, Error> {
    let multi_tree = multi_tree
        .lock()
        .map_err(|e| Error::InternalError(e.to_string()))?;
    let proof = multi_tree
        .get_merkle_proof(info.prefix.as_ref(), info.key.clone())
        .map_err(|e| Error::InternalError(e.to_string()))?;
    log::info!(
        "{:?}",
        format!("[Get Merkle Proof] info: {:?}, proof: {:?}", info, proof)
    );
    Ok(HttpResponse::Ok().json(proof))
}

#[get("/get_next_root")]
async fn get_next_root(
    multi_tree: web::Data<Mutex<MultiSMTStore<SMTKey, SMTValue, Keccak256Hasher>>>,
    info: web::Json<ReqNextRoot<SMTKey, SMTValue>>,
) -> Result<HttpResponse, Error> {
    let multi_tree = multi_tree
        .lock()
        .map_err(|e| Error::InternalError(e.to_string()))?;
    let old_proof = multi_tree
        .get_merkle_proof_old(
            info.prefix.as_ref(),
            info.keys
                .iter()
                .map(|kv| kv.0.clone())
                .collect::<Vec<SMTKey>>(),
        )
        .map_err(|e| Error::InternalError(e.to_string()))?;
    let next_root = multi_tree
        .get_next_root(old_proof, info.keys.clone())
        .map_err(|e| Error::InternalError(e.to_string()))?;
    log::info!(
        "{:?}",
        format!(
            "[Get Next Root] info: {:?}, next root: {:?}",
            info, next_root
        )
    );
    Ok(HttpResponse::Ok().json(next_root))
}

#[get("/get_root")]
async fn get_root(
    multi_tree: web::Data<Mutex<MultiSMTStore<SMTKey, SMTValue, Keccak256Hasher>>>,
    info: web::Query<ReqRoot>,
) -> Result<HttpResponse, Error> {
    let multi_tree = multi_tree
        .lock()
        .map_err(|e| Error::InternalError(e.to_string()))?;
    let root = multi_tree
        .get_root(format!("{}", info.prefix.clone()).as_ref())
        .map_err(|e| Error::InternalError(e.to_string()))?;
    log::info!(
        "{:?}",
        format!("[Get Root] info: {:?}, root: {:?}", info, root)
    );
    Ok(HttpResponse::Ok().json(root))
}

#[get("/get_value")]
async fn get_value(
    multi_tree: web::Data<Mutex<MultiSMTStore<SMTKey, SMTValue, Keccak256Hasher>>>,
    info: web::Json<ReqByKey<SMTKey>>,
) -> Result<HttpResponse, Error> {
    let multi_tree = multi_tree
        .lock()
        .map_err(|e| Error::InternalError(e.to_string()))?;
    let value = multi_tree
        .get_value(info.prefix.as_ref(), info.key.clone())
        .map_err(|e| Error::InternalError(e.to_string()))?;
    log::info!(
        "{:?}",
        format!("[Get Value] info: {:?}, value: {:?}", info, value)
    );
    Ok(HttpResponse::Ok().json(value))
}

#[post("/verify")]
async fn verify(multi_tree: web::Data<Mutex<MultiSMTStore<SMTKey, SMTValue, Keccak256Hasher>>>, 
    info: web::Json<Proof<SMTKey, SMTValue>>) -> Result<HttpResponse, Error> {
    let multi_tree = multi_tree
    .lock()
    .map_err(|e| Error::InternalError(e.to_string()))?;
    let res = multi_tree.verify(Proof {
        key: info.key.clone(),
        value: info.value.clone(),
        leave_bitmap: info.leave_bitmap,
        siblings: info.siblings.clone(),
        root: info.root,
    });
    log::info!(
        "{:?}",
        format!("[Verify] info: {:?}, res: {:?}", info, res)
    );
    Ok(HttpResponse::Ok().json(res))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let base_path = "./db";
    let multi_tree = web::Data::new(Mutex::new(
        MultiSMTStore::<SMTKey, SMTValue, Keccak256Hasher>::open(Path::new(base_path)).unwrap(),
    ));

    Logger::try_with_str("info")
        .unwrap()
        .log_to_file(flexi_logger::FileSpec::default().directory("target/logs"))
        .write_mode(WriteMode::BufferAndFlush)
        .rotate(
            flexi_logger::Criterion::Age(Age::Day),
            Naming::TimestampsDirect,
            Cleanup::Never,
        ) 
        .append()
        .log_to_stdout()
        .start()
        .unwrap();

    let app = HttpServer::new(move || {
        App::new()
            .app_data(multi_tree.clone())
            .service(update)
            .service(get_merkle_proof)
            .service(get_next_root)
            .service(get_root)
            .service(verify)
            .service(get_value)
            .service(test)
            
    })
    .shutdown_timeout(30)
    .bind(("127.0.0.1", 8080))?
    .run();

    let graceful_shutdown = async {
        ctrl_c().await.expect("Failed to listen for event");
        println!("Received CTRL-C, shutting down gracefully...");
    };

    let graceful_shutdown_task = tokio::spawn(graceful_shutdown);

    let result = tokio::select! {
        _ = app => Ok(()),
        _ = graceful_shutdown_task => Ok(()),
    };

    result
}
