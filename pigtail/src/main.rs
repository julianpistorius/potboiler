#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate log4rs;
extern crate r2d2;
extern crate r2d2_postgres;
extern crate iron;
extern crate persistent;
extern crate potboiler_common;
extern crate logger;
extern crate serde_json;
extern crate hyper;
extern crate router;
extern crate uuid;

use iron::prelude::{Chain, Iron, IronError, IronResult, Request, Response};
use iron::status;
use logger::Logger;
use persistent::Read as PRead;
use potboiler_common::db;
use std::env;
use std::io::Read;
use std::ops::Deref;

mod types;

pub type PostgresConnection = r2d2::PooledConnection<r2d2_postgres::PostgresConnectionManager>;

lazy_static! {
    static ref SERVER_URL: String = env::var("SERVER_URL").expect("Needed SERVER_URL");
}

fn json_from_body(req: &mut Request) -> IronResult<serde_json::Value> {
    let body_string = {
        let mut body = String::new();
        req.body.read_to_string(&mut body).expect("could read from body");
        body
    };
    let json: serde_json::Value = match serde_json::de::from_str(&body_string) {
        Ok(val) => val,
        Err(err) => return Err(IronError::new(err, (status::BadRequest, "Bad JSON"))),
    };
    return Ok(json);
}

fn add_queue_operation(op: types::QueueOperation) -> IronResult<Response> {
    let client = hyper::client::Client::new();
    let res = client.post(SERVER_URL.deref())
        .body(&serde_json::ser::to_string(&op).unwrap())
        .send()
        .expect("sender ok");
    assert_eq!(res.status, hyper::status::StatusCode::Created);
    Ok(Response::with(status::NoContent))
}

fn iron_json_error(se: serde_json::Error) -> iron::IronError {
    let desc = format!("{:?}", se);
    return IronError::new(se, (status::BadRequest, desc));
}

fn create_queue(req: &mut Request) -> IronResult<Response> {
    let json = try!(json_from_body(req));
    let op = try!(serde_json::from_value::<types::QueueCreate>(json).map_err(iron_json_error));
    return add_queue_operation(types::QueueOperation::Create(op));
}

fn new_event(req: &mut Request) -> IronResult<Response> {
    let json = try!(json_from_body(req));
    info!("body: {:?}", json);
    Ok(Response::with(status::NoContent))
}

fn make_queue_table(conn: &PostgresConnection) {
    conn.execute("CREATE TABLE IF NOT EXISTS queues (key VARCHAR(1024) PRIMARY KEY, config JSONB)",
                 &[])
        .expect("make queue table worked");
}

fn main() {
    log4rs::init_file("log.yaml", Default::default()).expect("log config ok");
    let client = hyper::client::Client::new();

    let mut map: serde_json::Map<String, String> = serde_json::Map::new();
    let host: &str = &env::var("HOST").unwrap_or("localhost".to_string());
    map.insert("url".to_string(),
               format!("http://{}:8000/event", host).to_string());
    let res = client.post(&format!("{}/register", SERVER_URL.deref()))
        .body(&serde_json::ser::to_string(&map).unwrap())
        .send()
        .expect("Register ok");
    assert_eq!(res.status, hyper::status::StatusCode::NoContent);

    let db_url: &str = &env::var("DATABASE_URL").expect("Needed DATABASE_URL");
    let pool = db::get_pool(db_url);
    let conn = pool.get().unwrap();
    make_queue_table(&conn);
    let (logger_before, logger_after) = Logger::new(None);
    let mut router = router::Router::new();
    router.post("/create", create_queue);
    router.post("/event", new_event);
    let mut chain = Chain::new(router);
    chain.link_before(logger_before);
    chain.link_after(logger_after);
    chain.link(PRead::<db::PostgresDB>::both(pool));
    info!("Pigtail booted");
    Iron::new(chain).http("0.0.0.0:8000").unwrap();
}
