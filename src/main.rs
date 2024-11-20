use std::{collections::HashMap, env};

use axum::{
    body::Body,
    http::{HeaderValue, Method, Response, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use tracing::debug;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

static LIST_DRIVERS_URL: &str =
    "https://order.printfactory.cloud/PF/_driverList.asp?Product=PrintFactory";

static GET_PRICES_URL: &str = "https://order.printfactory.cloud/PF/_prices.asp";

struct Error(anyhow::Error);

impl IntoResponse for Error {
    fn into_response(self) -> Response<Body> {
        (StatusCode::INTERNAL_SERVER_ERROR, self.0.to_string()).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for Error {
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();

    let host = match env::var("HOST") {
        Ok(host) => host,
        Err(_) => "localhost".into(),
    };

    let port = match env::var("PORT") {
        Ok(port) => port.parse().expect("should be an unsigned integer"),
        Err(_) => 3000,
    };

    let cors = CorsLayer::new()
        .allow_origin(
            "http://localhost:3000"
                .parse::<HeaderValue>()
                .expect("should parse CORS origin"),
        )
        .allow_origin(
            "https://app.ripee.fr"
                .parse::<HeaderValue>()
                .expect("should parse CORS origin"),
        )
        .allow_methods([Method::GET]);

    let app = Router::new()
        .route("/drivers", get(list_drivers))
        .route("/prices", post(get_prices))
        .layer(cors);

    debug!("starting server {host} at port {port}…");

    let listener = TcpListener::bind((host, port))
        .await
        .expect("should start TCP listener");

    axum::serve(listener, app)
        .await
        .expect("should start TCP server")
}

#[derive(Serialize, Deserialize)]
#[serde(transparent)]
struct Drivers(Vec<Driver>);

#[derive(Serialize, Deserialize)]
#[serde(rename_all(deserialize = "PascalCase"))]
struct Driver {
    name: String,
    code: String,
}

async fn list_drivers() -> Result<Json<Drivers>, Error> {
    let res = reqwest::get(LIST_DRIVERS_URL).await?;
    let bytes = res.bytes().await?;
    let drivers: Drivers = serde_json::from_slice(&bytes.slice(..))?;
    Ok(Json(drivers))
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
enum Plan {
    Connect,
    Production,
    #[serde(other)]
    Other,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetPricesResponse {
    r#type: String,
    results: Vec<(Plan, i16, f32, f32)>,
}

#[derive(Serialize, Deserialize)]
#[serde(transparent)]
struct Products(HashMap<String, usize>);

#[derive(Default, Serialize, Deserialize)]
struct Prices {
    yearly: PlanPrice,
    monthly: PlanPrice,
}

#[derive(Default, Serialize, Deserialize)]
struct PlanPrice {
    connect: usize,
    production: usize,
}

async fn get_prices(Json(products): Json<Products>) -> Result<Json<Prices>, Error> {
    let products = products.0.into_iter().collect::<Vec<_>>();
    let res = Client::new()
        .post(GET_PRICES_URL)
        .body(
            json!({
                "Product": "PrintFactory",
                "Currency": "EUR",
                "Products": products,
                "Country": "",
                "Dealer": null,
            })
            .to_string(),
        )
        .send()
        .await?;
    let bytes = res.bytes().await?;
    let res: GetPricesResponse = serde_json::from_slice(&bytes.slice(..))?;

    let prices = res
        .results
        .into_iter()
        .fold(Prices::default(), |mut prices, (plan, a, _b, c)| {
            match plan {
                Plan::Connect if a == 30 => {
                    prices.monthly.connect = (c * 100.0).round() as usize;
                }
                Plan::Connect if a == 365 => {
                    prices.yearly.connect = ((c / 12.0) * 100.0).round() as usize;
                }
                Plan::Production if a == 30 => {
                    prices.monthly.production = (c * 100.0).round() as usize;
                }
                Plan::Production if a == 365 => {
                    prices.yearly.production = ((c / 12.0) * 100.0).round() as usize;
                }
                Plan::Production => {}
                _ => {}
            };
            prices
        });

    Ok(Json(prices))
}
