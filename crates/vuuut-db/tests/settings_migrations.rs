use vuuut_db::{SettingsCache, connect_test, repos, seed};
use serde_json::json;

#[tokio::test]
async fn fresh_install_has_valid_settings_and_public_paths() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    seed::seed_if_empty(&db).await.unwrap();

    for setting in repos::settings::all(&db).await.unwrap() {
        match setting.value_type.as_str() {
            "boolean" => assert!(
                matches!(setting.value.as_str(), "" | "true" | "false" | "1" | "0"),
                "{} has an invalid boolean default",
                setting.key
            ),
            "number" => assert!(
                setting.value.is_empty() || setting.value.parse::<f64>().is_ok(),
                "{} has an invalid number default",
                setting.key
            ),
            "string" | "array" | "object" => {}
            other => panic!("{} has an unknown settings type {other:?}", setting.key),
        }
    }

    let settings = SettingsCache::load(&db).await.unwrap();
    assert_eq!(settings.get_str("search_engine").as_deref(), Some("sodo"));
    assert_eq!(settings.get("directory_moderate"), Some(json!(true)));
    assert_eq!(settings.get("directory_per_page"), Some(json!(10)));
    assert_eq!(settings.get_str("directory_base_path").as_deref(), Some("directory"));
    assert_eq!(settings.get_str("directory_listing_path").as_deref(), Some(""));
    assert_eq!(settings.get("cache_ttl"), Some(json!(300)));
    assert_eq!(settings.get_str("ecom_base_path").as_deref(), Some("shop"));
    assert_eq!(settings.get("ecom_per_page"), Some(json!(10)));
    assert_eq!(
        settings.get_str("ecom_product_path").as_deref(),
        Some("shop/products")
    );
}

#[tokio::test]
async fn upgrade_repairs_reversed_rows_and_keeps_saved_preferences() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    for (key, value, kind) in [
        ("cache_ttl", "number", "300"),
        ("directory_moderate", "boolean", "true"),
        ("cache_pages", "true", "boolean"),
        ("directory_title", "Local listings", "string"),
        ("ecom_enabled", "true", "boolean"),
        ("ecom_base_path", "store", "string"),
    ] {
        sqlx::query("UPDATE settings SET value = ?, type = ? WHERE `key` = ?")
            .bind(value)
            .bind(kind)
            .bind(key)
            .execute(&db)
            .await
            .unwrap();
    }

    // Both embedded migrations use the same SQL, including on MySQL.
    let migration =
        include_str!("../../../migrations/sqlite/0036_repair_caching_directory_settings.sql");
    assert_eq!(
        migration,
        include_str!("../../../migrations/mysql/0036_repair_caching_directory_settings.sql")
    );
    sqlx::raw_sql(migration).execute(&db).await.unwrap();
    let settings = SettingsCache::load(&db).await.unwrap();
    assert_eq!(settings.get("cache_ttl"), Some(json!(300)));
    assert_eq!(settings.get("directory_moderate"), Some(json!(true)));
    assert!(settings.get_bool("cache_pages"));
    assert_eq!(
        settings.get_str("directory_title").as_deref(),
        Some("Local listings")
    );
    assert!(settings.get_bool("ecom_enabled"));
    assert_eq!(settings.get_str("ecom_base_path").as_deref(), Some("store"));
}

#[tokio::test]
async fn upgrade_adds_directory_addresses_and_preserves_saved_paths() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    sqlx::query("DELETE FROM settings WHERE `key` = 'directory_listing_path'")
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("UPDATE settings SET value = 'town-guide' WHERE `key` = 'directory_base_path'")
        .execute(&db)
        .await
        .unwrap();

    use vuuut_core::database::Engine;

    let migration = match vuuut_core::database::engine() {
        Engine::Sqlite => include_str!("../../../migrations/sqlite/0037_directory_paths.sql"),
        Engine::MySql => include_str!("../../../migrations/mysql/0037_directory_paths.sql"),
    };
    sqlx::raw_sql(migration).execute(&db).await.unwrap();
    let settings = SettingsCache::load(&db).await.unwrap();
    assert_eq!(settings.get_str("directory_base_path").as_deref(), Some("town-guide"));
    assert_eq!(settings.get_str("directory_listing_path").as_deref(), Some(""));
}


#[tokio::test]
async fn upgrades_change_original_page_sizes_and_preserve_saved_choices() {
    let test = connect_test().await.unwrap();
    let db = test.pool();
    for (key, sqlite, mysql) in [
        (
            "ecom_per_page",
            include_str!("../../../migrations/sqlite/0038_ecom_page_size_default.sql"),
            include_str!("../../../migrations/mysql/0038_ecom_page_size_default.sql"),
        ),
        (
            "directory_per_page",
            include_str!("../../../migrations/sqlite/0039_directory_page_size_default.sql"),
            include_str!("../../../migrations/mysql/0039_directory_page_size_default.sql"),
        ),
    ] {
        assert_eq!(sqlite, mysql);
        for (value, updated_at, expected) in [
            ("24", "2026-01-01 00:00:00", 10),
            ("24", "2026-01-02 00:00:00", 24),
            ("12", "2026-01-02 00:00:00", 12),
        ] {
            sqlx::query("UPDATE settings SET value = ?, created_at = '2026-01-01 00:00:00', updated_at = ? WHERE `key` = ?")
                .bind(value)
                .bind(updated_at)
                .bind(key)
                .execute(&db)
                .await
                .unwrap();
            sqlx::raw_sql(sqlite).execute(&db).await.unwrap();
            let settings = SettingsCache::load(&db).await.unwrap();
            assert_eq!(settings.get(key), Some(json!(expected)));
        }
    }
}
