use prometheus_client::metrics::counter::Counter;
use prometheus_client::registry::Registry;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;

use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::encoding::EncodeLabelValue;
use prometheus_client::encoding::text::encode;
use std::sync::Mutex;


use actix_web::middleware::Compress;
use actix_web::{web, App, HttpResponse, HttpServer, Responder, Result};



#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct Labels {
  pub path: String,
}

pub struct Metrics {
}

pub struct AppState {
    pub registry: Registry,
    requests: Family<Labels, Gauge>,
}

pub async fn metrics_handler(state: web::Data<Mutex<AppState>>) -> Result<HttpResponse> {
    let state = state.lock().unwrap();
    let mut body = String::new();
    get_fdb_metrics(&state.requests).await.unwrap();
    encode(&mut body, &state.registry).unwrap();
    Ok(HttpResponse::Ok()
        .content_type("application/openmetrics-text; version=1.0.0; charset=utf-8")
        .body(body))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {

    let _network = unsafe { foundationdb::boot() };

    let metrics = web::Data::new(Metrics {
        
    });

    let mut state = AppState {
        registry: Registry::default(),
        requests: Family::default(),
    };
    state
        .registry
        .register("FDBMetrics", "FDBMetrics", state.requests.clone());

    let state = web::Data::new(Mutex::new(state));


    HttpServer::new(move || {
        App::new()
            .wrap(Compress::default())
            .app_data(metrics.clone())
            .app_data(state.clone())
            .service(web::resource("/metrics").route(web::get().to(metrics_handler)))
    })
    .bind(("0.0.0.0", 18090))?
    .run()
    .await

    //drop(network);
}


async fn get_fdb_metrics(family : &Family<Labels, Gauge>) -> foundationdb::FdbResult<()> {
    let db = foundationdb::Database::default()?;
    let metrics_key = b"\xff\xff/status/json";


    // read a value
    match db
        .run(|trx, _maybe_committed| async move { 
            let _ = trx.set_option(foundationdb::options::TransactionOption::SpecialKeySpaceRelaxed).unwrap();
            Ok(trx.get(metrics_key, false).await.unwrap()) 
        })
        .await
    {
        Ok(slice) => {
            // Convert the byte slice to a string and print it
            match std::str::from_utf8(slice.unwrap().as_ref()) {
                Ok(str_value) => 
                {
                    let state = json::parse(str_value).unwrap();
                    // flatten the json object to path: value
                    fn flatten_json(json: &json::JsonValue, prefix: String, flattened: &mut std::collections::HashMap<String, json::JsonValue>) {
                        match json {
                            json::JsonValue::Object(obj) => {
                                for (key, value) in obj.iter() {
                                    let new_prefix = if prefix.is_empty() {
                                        key.to_string()
                                    } else {
                                        format!("{}.{}", prefix, key)
                                    };
                                    flatten_json(value, new_prefix, flattened);
                                }
                            }
                            _ => {
                                flattened.insert(prefix, json.clone());
                            }
                        }
                    }
                    println!("state: {}", state);

                    let mut flattened = std::collections::HashMap::new();
                    flatten_json(&state, String::new(), &mut flattened);
                    for (key, value) in flattened.iter() {
                        println!("{}: {}", key, value);
                        match value {
                            json::JsonValue::Number(num) => {
                                family.get_or_create(
                                    &Labels { path: key.to_string() }
                                ).set(num.as_fixed_point_i64(2).unwrap());
                            }
                            _ => {}
                        }
                    }
                }
                    
                Err(_) => eprintln!("failed to convert byte slice to string"),
            }
        },
        Err(_) => eprintln!("cannot commit transaction"),
    }

    Ok(())
}