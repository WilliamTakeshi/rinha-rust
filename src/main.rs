use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{postgres::PgPoolOptions, PgPool};
use time::PrimitiveDateTime;
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use std::{str::FromStr, time::Duration};

#[derive(Serialize, Deserialize)]
struct Statement {
    #[serde(rename = "extrato")]
    balance: Balance,
    #[serde(rename = "ultimas_transacoes")]
    transactions: Vec<Transaction>,
}

#[derive(Serialize, Deserialize)]
struct Balance {
    #[serde(rename = "total")]
    total: i64,
    #[serde(rename = "limite")]
    credit_limit: u64,
    #[serde(rename = "data_extrato")]
    balance_date: PrimitiveDateTime,
}

#[derive(Deserialize, Serialize)]
struct PostTransaction {
    #[serde(rename = "valor")]
    value: i32,
    #[serde(rename = "tipo")]
    kind: TransactionKind,
    #[serde(rename = "descricao")]
    description: String,
}

#[derive(Deserialize, Serialize)]
struct TransactionResponse {
    #[serde(rename = "saldo")]
    balance: u64,
    #[serde(rename = "limite")]
    credit_limit: u64,
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

#[derive(Deserialize, Serialize)]
enum TransactionKind {
    #[serde(rename = "c")]
    Credit,
    #[serde(rename = "d")]
    Debit,
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
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "example_tokio_postgres=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let db_connection_str = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://rinha:rinha@localhost/rinha".to_string());

    // set up connection pool
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(3))
        .connect(&db_connection_str)
        .await
        .expect("can't connect to database");

    sqlx::migrate!().run(&pool).await.unwrap();

    // build our application with some routes
    let app = Router::new()
        .route("/", get(hello_world))
        // .route("/clientes/:id/extrato", get(statement))
        .route("/clientes/:id/transacoes", post(insert_transaction))
        .with_state(pool);

    // run it with hyper
    let listener = TcpListener::bind("127.0.0.1:3000").await.unwrap();
    tracing::debug!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

async fn hello_world() -> String {
    "Hello, World!".to_string()
}

// async fn statement(
//     Path(wallet_id): Path<i32>,
//     State(pool): State<PgPool>,
// ) -> Result<Statement, (StatusCode, String)> {
//     sqlx::query_as!(
//         Transaction,
//         r#"
//         SELECT value, kind, description, inserted_at
//         FROM transactions
//         WHERE wallet_id = $1
//         "#,
//         wallet_id
//     )
//     .fetch_all(&pool)
//     .await
//     .map_err(internal_error)?
// }

async fn insert_transaction(
    Path(wallet_id): Path<i32>,
    State(pool): State<PgPool>,
    Json(post_transaction): Json<PostTransaction>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mut transaction = pool.begin().await.map_err(internal_error)?;

    let _ = sqlx::query!(
        r#"
        INSERT INTO transactions (wallet_id, value, kind, description) VALUES ($1, $2, $3, $4);
        "#,
        wallet_id,
        post_transaction.value,
        post_transaction.kind.to_string(),
        post_transaction.description
    )
    .execute(&mut *transaction)
    .await
    .map_err(internal_error)?;


    let updated_value = match post_transaction.kind {
        TransactionKind::Credit => post_transaction.value,
        TransactionKind::Debit => -post_transaction.value,
    };

    let res = sqlx::query!(
        r#"
        UPDATE wallets SET balance = balance + $1 WHERE id = $2 RETURNING balance, credit_limit;
        "#,
        updated_value,
        wallet_id
    )
    .fetch_one(&mut *transaction)
    .await
    .map_err(internal_error)?;

    transaction.commit().await.map_err(internal_error)?;

    println!("{:?}", res);

    Ok(Json(json!({
        "saldo": res.balance,
        "limite": res.credit_limit
    })))
}

fn internal_error<E>(err: E) -> (StatusCode, String)
where
    E: std::error::Error,
{
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
