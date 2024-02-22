use std::str::FromStr;

use axum::{routing::get, Router};
use serde::{Deserialize, Serialize};
use time::PrimitiveDateTime;
use tracing::subscriber::set_global_default;
use tracing_bunyan_formatter::{BunyanFormattingLayer, JsonStorageLayer};
use tracing_log::LogTracer;
use tracing_subscriber::{layer::SubscriberExt, EnvFilter, Registry};

#[derive(Deserialize, Serialize)]
struct PostTransaction {
    value: u64,
    kind: TransactionKind,
    description: String,
}

#[derive(Deserialize, Serialize)]
struct Transaction {
    #[serde(rename = "valor")]
    value: u64,
    #[serde(rename = "tipo")]
    kind: TransactionKind,
    #[serde(rename = "descricao")]
    description: String,
    #[serde(rename = "realizada_em")]
    inserted_at: PrimitiveDateTime,
}

enum TransactionKind {
    Credit,
    Debit,
}

impl Serialize for TransactionKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for TransactionKind {
    fn deserialize<D>(deserializer: D) -> Result<TransactionKind, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        TransactionKind::from_str(&s).map_err(serde::de::Error::custom)
    }
}

impl FromStr for TransactionKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "c" => Ok(TransactionKind::Credit),
            "d" => Ok(TransactionKind::Debit),
            _ => Err(format!("Invalid transaction kind: {}", s)),
        }
    }
}

impl ToString for TransactionKind {
    fn to_string(&self) -> String {
        match self {
            TransactionKind::Credit => "c".to_string(),
            TransactionKind::Debit => "d".to_string(),
        }
    }
}

#[tokio::main]
async fn main() {
    LogTracer::init().expect("Failed to set logger");
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug"));
    let formatting_layer = BunyanFormattingLayer::new("rinha-rust".into(), std::io::stdout);
    let subscriber = Registry::default()
        .with(env_filter)
        .with(JsonStorageLayer)
        .with(formatting_layer);
    set_global_default(subscriber).expect("Failed to set subscriber");

    let app = Router::new().route("/", get(root));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::debug!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

// basic handler that responds with a static string
async fn root() -> &'static str {
    "Hello, World!"
}
