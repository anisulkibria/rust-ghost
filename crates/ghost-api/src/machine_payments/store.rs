//! Atomic durable nonce storage for the native MPP SDK.
use mpp::store::{Store, StoreError};
use serde_json::Value;
use std::{future::Future, pin::Pin};
type Response<'a, T> = Pin<Box<dyn Future<Output = Result<T, StoreError>> + Send + 'a>>;
#[derive(Clone)]
pub struct SqliteStore(pub sqlx::SqlitePool);
fn error(error: sqlx::Error) -> StoreError {
    StoreError::Internal(error.to_string())
}
fn canonical(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: std::collections::BTreeMap<_, _> =
                map.into_iter().map(|(k, v)| (k, canonical(v))).collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonical).collect()),
        other => other,
    }
}
fn encoded(value: Value) -> String {
    canonical(value).to_string()
}
impl Store for SqliteStore {
    fn get(&self, key: &str) -> Response<'_, Option<Value>> {
        let key = key.to_owned();
        Box::pin(async move {
            let value: Option<String> =
                sqlx::query_scalar("SELECT value FROM machine_payment_replay_store WHERE key=?")
                    .bind(key)
                    .fetch_optional(&self.0)
                    .await
                    .map_err(error)?;
            value
                .map(|value| {
                    serde_json::from_str(&value)
                        .map_err(|e| StoreError::Serialization(e.to_string()))
                })
                .transpose()
        })
    }
    fn put(&self, key: &str, value: Value) -> Response<'_, ()> {
        let key = key.to_owned();
        Box::pin(async move {
            sqlx::query("INSERT INTO machine_payment_replay_store (key,value) VALUES (?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value").bind(key).bind(encoded(value)).execute(&self.0).await.map_err(error)?;
            Ok(())
        })
    }
    fn delete(&self, key: &str) -> Response<'_, ()> {
        let key = key.to_owned();
        Box::pin(async move {
            sqlx::query("DELETE FROM machine_payment_replay_store WHERE key=?")
                .bind(key)
                .execute(&self.0)
                .await
                .map_err(error)?;
            Ok(())
        })
    }
    fn put_if_absent(&self, key: &str, value: Value) -> Response<'_, bool> {
        let key = key.to_owned();
        Box::pin(async move {
            Ok(sqlx::query(
                "INSERT OR IGNORE INTO machine_payment_replay_store (key,value) VALUES (?,?)",
            )
            .bind(key)
            .bind(encoded(value))
            .execute(&self.0)
            .await
            .map_err(error)?
            .rows_affected()
                == 1)
        })
    }
    fn compare_exchange(
        &self,
        key: &str,
        expected: Option<Value>,
        value: Option<Value>,
    ) -> Response<'_, bool> {
        let key = key.to_owned();
        Box::pin(async move {
            let changed =
                match (expected, value) {
                    (None, Some(value)) => return self.put_if_absent(&key, value).await,
                    (None, None) => return sqlx::query_scalar(
                        "SELECT NOT EXISTS(SELECT 1 FROM machine_payment_replay_store WHERE key=?)",
                    )
                    .bind(key)
                    .fetch_one(&self.0)
                    .await
                    .map_err(error),
                    (Some(expected), Some(value)) => sqlx::query(
                        "UPDATE machine_payment_replay_store SET value=? WHERE key=? AND value=?",
                    )
                    .bind(encoded(value))
                    .bind(key)
                    .bind(encoded(expected))
                    .execute(&self.0)
                    .await
                    .map_err(error)?
                    .rows_affected(),
                    (Some(expected), None) => sqlx::query(
                        "DELETE FROM machine_payment_replay_store WHERE key=? AND value=?",
                    )
                    .bind(key)
                    .bind(encoded(expected))
                    .execute(&self.0)
                    .await
                    .map_err(error)?
                    .rows_affected(),
                };
            Ok(changed == 1)
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    async fn replay_reservations_and_cas_are_atomic() {
        let db = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE machine_payment_replay_store (key TEXT PRIMARY KEY,value TEXT NOT NULL)",
        )
        .execute(&db)
        .await
        .unwrap();
        let store = SqliteStore(db);
        assert!(store
            .put_if_absent("nonce", json!({"used":true}))
            .await
            .unwrap());
        assert!(!store
            .put_if_absent("nonce", json!({"used":false}))
            .await
            .unwrap());
        assert!(!store
            .compare_exchange("nonce", Some(json!({"used":false})), None)
            .await
            .unwrap());
        assert!(store
            .compare_exchange(
                "nonce",
                Some(json!({"used":true})),
                Some(json!({"used":true,"settled":true}))
            )
            .await
            .unwrap());
        assert_eq!(
            store.get("nonce").await.unwrap(),
            Some(json!({"settled":true,"used":true}))
        );
    }
}
