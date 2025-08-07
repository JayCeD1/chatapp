use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::State;

#[derive(Serialize, Deserialize)]
pub struct User {
    pub id: Option<i64>,
    pub name: String,
    pub email: String,
}

#[derive(Serialize)]
pub struct InsertResult {
    pub rows_affected: u64,
    pub last_insert_id: i64,
}

#[tauri::command]
pub async fn create_user(
    db: State<'_, SqlitePool>,
    name: String,
    email: String,
) -> Result<InsertResult, String> {
    let result = sqlx::query("INSERT INTO users (name, email) VALUES ($1, $2)")
        .bind(&name)
        .bind(&email)
        .execute(&*db)
        .await
        .map_err(|e| format!("Failed to insert user: {}", e))?;

    Ok(InsertResult {
        rows_affected: result.rows_affected(),
        last_insert_id: result.last_insert_rowid(),
    })
}

#[tauri::command]
pub async fn get_users(db: State<'_, SqlitePool>) -> Result<Vec<User>, String> {
    let result = sqlx::query("SELECT id, name, email FROM users")
        .fetch_all(&*db)
        .await
        .map_err(|e| format!("Failed to get users: {}", e))?;

    let mut users = Vec::new();
    for row in result {
        users.push( User {
            id: row.get::<Option<i64>, _>("id"),
            name: row.get::<String, _>("name"),
            email: row.get::<String, _>("email"),
        });

    }
        Ok(users)
}

#[tauri::command]
pub async fn get_user_by_id(db: State<'_, SqlitePool>, id: i64) -> Result<Option<User>, String> {
    let result = sqlx::query("SELECT id, name, email FROM users WHERE id = $1")
        .bind(&id)
        .fetch_optional(&*db)
        .await
        .map_err(|e| format!("Failed to get user by id: {}", e))?;

    if let Some(row) = result {
        Ok(Some(User {
            id: row.get::<Option<i64>, _>("id"),
            name: row.get::<String, _>("name"),
            email: row.get::<String, _>("email"),
        }))
    }else { Ok(None) }
}