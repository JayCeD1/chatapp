use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::State;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
pub struct User {
    pub id: Option<i64>,
    pub name: String,
    pub email: String,
    pub department_id: Option<i64>,
    pub department_name: Option<String>,
    pub is_online: bool,
    pub last_seen: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Department {
    pub id: Option<i64>,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct ChatRoom {
    pub id: Option<i64>,
    pub name: String,
    pub description: Option<String>,
    pub department_id: Option<i64>,
    pub department_name: Option<String>,
    pub is_private: bool,
    pub user_count: Option<i64>,
}

#[derive(Serialize, Deserialize)]
pub struct Message {
    pub id: Option<i64>,
    pub message_id: Option<String>,
    pub room_id: i64,
    pub user_id: i64,
    pub username: String,
    pub message: String,
    pub message_type: String,
    pub is_emoji: bool,
    pub created_at: String,
    pub edited_at: Option<String>,
    pub deleted_at: Option<String>,
}

#[derive(Serialize)]
pub struct InsertResult {
    pub rows_affected: u64,
    pub last_insert_id: i64,
}

// User management
#[tauri::command]
pub async fn upsert_user(
    db: State<'_, SqlitePool>,
    name: String,
    email: String,
    department_id: Option<i64>,
) -> Result<User, String> {
    // Normalize + validate so identity (the broadcast/attribution key) stays clean:
    // trim/lowercase email, reject blank/oversized values.
    let name = name.trim().to_string();
    let email = email.trim().to_lowercase();
    if name.is_empty() || name.chars().count() > 64 {
        return Err("Name must be between 1 and 64 characters".to_string());
    }
    if email.is_empty() || email.len() > 254 || !email.contains('@') {
        return Err("A valid email address is required".to_string());
    }

    //Try find existing
    if let Some(_row) = sqlx::query(
        "SELECT u.id, u.name, u.email, u.department_id, u.is_online, u.last_seen,
                d.name as department_name
         FROM users u LEFT JOIN departments d ON u.department_id = d.id
         WHERE u.email = $1",
    )
    .bind(&email)
    .fetch_optional(&*db)
    .await
    .map_err(|e| e.to_string())?
    {
        // Optionally update display name/department if changed
        sqlx::query("UPDATE users SET name=$1, department_id=$2 WHERE email=$3")
            .bind(&name)
            .bind(&department_id)
            .bind(&email)
            .execute(&*db)
            .await
            .map_err(|e| e.to_string())?;
    } else {
        // Create
        sqlx::query("INSERT INTO users (name, email, department_id) VALUES ($1, $2, $3)")
            .bind(&name)
            .bind(&email)
            .bind(&department_id)
            .execute(&*db)
            .await
            .map_err(|e| e.to_string())?;
    }

    // Return the user
    let row = sqlx::query(
        "SELECT u.id, u.name, u.email, u.department_id, u.is_online, u.last_seen,
                d.name as department_name
         FROM users u LEFT JOIN departments d ON u.department_id = d.id
         WHERE u.email = $1",
    )
    .bind(&email)
    .fetch_one(&*db)
    .await
    .map_err(|e| e.to_string())?;

    Ok(User {
        id: row.get::<Option<i64>, _>("id"),
        name: row.get::<String, _>("name"),
        email: row.get::<String, _>("email"),
        department_id: row.get::<Option<i64>, _>("department_id"),
        department_name: row.get::<Option<String>, _>("department_name"),
        is_online: row.get::<bool, _>("is_online"),
        last_seen: row.get::<Option<String>, _>("last_seen"),
    })
}

#[tauri::command]
pub async fn create_user(
    db: State<'_, SqlitePool>,
    name: String,
    email: String,
    department_id: Option<i64>,
) -> Result<InsertResult, String> {
    let result = sqlx::query("INSERT INTO users (name, email, department_id) VALUES ($1, $2, $3)")
        .bind(&name)
        .bind(&email)
        .bind(&department_id)
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
    let result = sqlx::query(
        "SELECT u.id, u.name, u.email, u.department_id, u.is_online, u.last_seen, d.name as department_name 
         FROM users u 
         LEFT JOIN departments d ON u.department_id = d.id 
         ORDER BY u.name"
    )
        .fetch_all(&*db)
        .await
        .map_err(|e| format!("Failed to get users: {}", e))?;

    let mut users = Vec::new();
    for row in result {
        users.push(User {
            id: row.get::<Option<i64>, _>("id"),
            name: row.get::<String, _>("name"),
            email: row.get::<String, _>("email"),
            department_id: row.get::<Option<i64>, _>("department_id"),
            department_name: row.get::<Option<String>, _>("department_name"),
            is_online: row.get::<bool, _>("is_online"),
            last_seen: row.get::<Option<String>, _>("last_seen"),
        });
    }
    Ok(users)
}

#[tauri::command]
pub async fn get_user_by_id(db: State<'_, SqlitePool>, id: i64) -> Result<Option<User>, String> {
    let result = sqlx::query(
        "SELECT u.id, u.name, u.email, u.department_id, u.is_online, u.last_seen, d.name as department_name 
         FROM users u 
         LEFT JOIN departments d ON u.department_id = d.id 
         WHERE u.id = $1"
    )
        .bind(&id)
        .fetch_optional(&*db)
        .await
        .map_err(|e| format!("Failed to get user by id: {}", e))?;

    if let Some(row) = result {
        Ok(Some(User {
            id: row.get::<Option<i64>, _>("id"),
            name: row.get::<String, _>("name"),
            email: row.get::<String, _>("email"),
            department_id: row.get::<Option<i64>, _>("department_id"),
            department_name: row.get::<Option<String>, _>("department_name"),
            is_online: row.get::<bool, _>("is_online"),
            last_seen: row.get::<Option<String>, _>("last_seen"),
        }))
    } else {
        Ok(None)
    }
}

#[tauri::command]
pub async fn update_user_online_status(
    db: State<'_, SqlitePool>,
    user_id: i64,
    is_online: bool,
) -> Result<(), String> {
    sqlx::query("UPDATE users SET is_online = $1, last_seen = CURRENT_TIMESTAMP WHERE id = $2")
        .bind(&is_online)
        .bind(&user_id)
        .execute(&*db)
        .await
        .map_err(|e| format!("Failed to update user status: {}", e))?;

    Ok(())
}

// Department management
#[tauri::command]
pub async fn get_departments(db: State<'_, SqlitePool>) -> Result<Vec<Department>, String> {
    let result = sqlx::query("SELECT id, name, description FROM departments ORDER BY name")
        .fetch_all(&*db)
        .await
        .map_err(|e| format!("Failed to get departments: {}", e))?;

    let mut departments = Vec::new();
    for row in result {
        departments.push(Department {
            id: row.get::<Option<i64>, _>("id"),
            name: row.get::<String, _>("name"),
            description: row.get::<Option<String>, _>("description"),
        });
    }
    Ok(departments)
}

// Chat room management
#[tauri::command]
pub async fn get_chat_rooms(db: State<'_, SqlitePool>) -> Result<Vec<ChatRoom>, String> {
    let result = sqlx::query(
        "SELECT
  cr.id,
  cr.name,
  cr.description,
  cr.department_id,
  cr.is_private,
  d.name AS department_name,
  COALESCE(urc.user_count, 0) AS user_count
FROM chat_rooms cr
LEFT JOIN departments d
  ON cr.department_id = d.id
LEFT JOIN (
  SELECT room_id, COUNT(DISTINCT user_id) AS user_count
  FROM user_rooms
  WHERE is_active = 1
  GROUP BY room_id
) urc
  ON urc.room_id = cr.id
ORDER BY cr.name
",
    )
    .fetch_all(&*db)
    .await
    .map_err(|e| format!("Failed to get chat rooms: {}", e))?;

    let mut rooms = Vec::new();
    for row in result {
        rooms.push(ChatRoom {
            id: row.get::<Option<i64>, _>("id"),
            name: row.get::<String, _>("name"),
            description: row.get::<Option<String>, _>("description"),
            department_id: row.get::<Option<i64>, _>("department_id"),
            department_name: row.get::<Option<String>, _>("department_name"),
            is_private: row.get::<bool, _>("is_private"),
            user_count: row.get::<Option<i64>, _>("user_count"),
        });
    }
    Ok(rooms)
}

#[tauri::command]
pub async fn get_rooms_by_department(
    db: State<'_, SqlitePool>,
    department_id: i64,
) -> Result<Vec<ChatRoom>, String> {
    let result = sqlx::query(
        "SELECT
  cr.id,
  cr.name,
  cr.description,
  cr.department_id,
  cr.is_private,
  d.name AS department_name,
  COALESCE(urc.user_count, 0) AS user_count
FROM chat_rooms cr
LEFT JOIN departments d
  ON cr.department_id = d.id
LEFT JOIN (
  SELECT room_id, COUNT(DISTINCT user_id) AS user_count
  FROM user_rooms
  WHERE is_active = 1
  GROUP BY room_id
) urc
  ON urc.room_id = cr.id
WHERE cr.department_id = $1
ORDER BY cr.name
",
    )
    .bind(&department_id)
    .fetch_all(&*db)
    .await
    .map_err(|e| format!("Failed to get rooms by department: {}", e))?;

    let mut rooms = Vec::new();
    for row in result {
        rooms.push(ChatRoom {
            id: row.get::<Option<i64>, _>("id"),
            name: row.get::<String, _>("name"),
            description: row.get::<Option<String>, _>("description"),
            department_id: row.get::<Option<i64>, _>("department_id"),
            department_name: row.get::<Option<String>, _>("department_name"),
            is_private: row.get::<bool, _>("is_private"),
            user_count: row.get::<Option<i64>, _>("user_count"),
        });
    }
    Ok(rooms)
}

#[tauri::command]
pub async fn create_room(
    db: State<'_, SqlitePool>,
    name: String,
    description: Option<String>,
    department_id: Option<i64>,
    is_private: Option<bool>,
    created_by: Option<i64>,
) -> Result<ChatRoom, String> {
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 64 {
        return Err("Channel name must be between 1 and 64 characters".to_string());
    }
    let is_private = is_private.unwrap_or(false);

    let result = sqlx::query(
        "INSERT INTO chat_rooms (name, description, department_id, is_private, created_by)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&name)
    .bind(&description)
    .bind(&department_id)
    .bind(&is_private)
    .bind(&created_by)
    .execute(&*db)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            "A channel with that name already exists".to_string()
        } else {
            format!("Failed to create channel: {}", e)
        }
    })?;

    let id = result.last_insert_rowid();
    let row = sqlx::query(
        "SELECT cr.id, cr.name, cr.description, cr.department_id, cr.is_private,
                d.name as department_name, 0 as user_count
         FROM chat_rooms cr LEFT JOIN departments d ON cr.department_id = d.id
         WHERE cr.id = $1",
    )
    .bind(&id)
    .fetch_one(&*db)
    .await
    .map_err(|e| e.to_string())?;

    Ok(ChatRoom {
        id: row.get::<Option<i64>, _>("id"),
        name: row.get::<String, _>("name"),
        description: row.get::<Option<String>, _>("description"),
        department_id: row.get::<Option<i64>, _>("department_id"),
        department_name: row.get::<Option<String>, _>("department_name"),
        is_private: row.get::<bool, _>("is_private"),
        user_count: row.get::<Option<i64>, _>("user_count"),
    })
}

#[tauri::command]
pub async fn join_room(
    db: State<'_, SqlitePool>,
    user_id: i64,
    room_id: i64,
) -> Result<(), String> {
    // Upsert without churning the PK / joined_at (INSERT OR REPLACE would delete+reinsert).
    sqlx::query(
        "INSERT INTO user_rooms (user_id, room_id, is_active) VALUES ($1, $2, 1)
         ON CONFLICT(user_id, room_id) DO UPDATE SET is_active = 1",
    )
    .bind(&user_id)
    .bind(&room_id)
    .execute(&*db)
    .await
    .map_err(|e| format!("Failed to join room: {}", e))?;

    Ok(())
}

#[tauri::command]
pub async fn leave_room(
    db: State<'_, SqlitePool>,
    user_id: i64,
    room_id: i64,
) -> Result<(), String> {
    sqlx::query("UPDATE user_rooms SET is_active = 0 WHERE user_id = $1 AND room_id = $2")
        .bind(&user_id)
        .bind(&room_id)
        .execute(&*db)
        .await
        .map_err(|e| format!("Failed to leave room: {}", e))?;

    Ok(())
}

// Message management
#[tauri::command]
pub async fn save_message(
    db: State<'_, SqlitePool>,
    room_id: i64,
    user_id: i64,
    message: String,
    message_type: String,
    is_emoji: bool,
) -> Result<InsertResult, String> {
    // Generate a stable id so callers of this command also get idempotent inserts.
    let message_id = Uuid::new_v4().to_string();
    save_message_internal(
        &db,
        room_id,
        user_id,
        message,
        message_type,
        is_emoji,
        message_id,
    )
    .await
}

pub async fn save_message_internal(
    pool: &SqlitePool,
    room_id: i64,
    user_id: i64,
    message: String,
    message_type: String,
    is_emoji: bool,
    message_id: String,
) -> Result<InsertResult, String> {
    // ON CONFLICT(message_id) DO NOTHING makes retried/echoed saves idempotent.
    let result = sqlx::query(
        "INSERT INTO messages (room_id, user_id, message, message_type, is_emoji, message_id)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT(message_id) DO NOTHING",
    )
    .bind(&room_id)
    .bind(&user_id)
    .bind(&message)
    .bind(&message_type)
    .bind(&is_emoji)
    .bind(&message_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to save message: {}", e))?;

    Ok(InsertResult {
        rows_affected: result.rows_affected(),
        last_insert_id: result.last_insert_rowid(),
    })
}

#[tauri::command]
pub async fn get_room_messages(
    db: State<'_, SqlitePool>,
    room_id: i64,
    limit: Option<i64>,
    before_id: Option<i64>,
) -> Result<Vec<Message>, String> {
    let limit = limit.unwrap_or(50);

    // before_id = None → newest `limit`. before_id = Some(id) → the `limit` messages
    // immediately older than `id`. Order + paginate by `id` (the monotonic insertion
    // order) so the cursor and the sort key always agree — ordering by the wall-clock
    // created_at would disagree with the `id < before_id` cursor under clock skew and
    // silently drop history.
    let result = sqlx::query(
        "SELECT m.id, m.message_id, m.room_id, m.user_id, m.message, m.message_type, m.is_emoji, m.created_at,
                m.edited_at, m.deleted_at, COALESCE(u.name, 'Unknown') as username
         FROM messages m
         LEFT JOIN users u ON m.user_id = u.id
         WHERE m.room_id = $1 AND ($2 IS NULL OR m.id < $2)
         ORDER BY m.id DESC
         LIMIT $3",
    )
    .bind(&room_id)
    .bind(&before_id)
    .bind(&limit)
    .fetch_all(&*db)
    .await
    .map_err(|e| format!("Failed to get room messages: {}", e))?;

    let mut messages = Vec::new();
    for row in result {
        messages.push(Message {
            id: row.get::<Option<i64>, _>("id"),
            message_id: row.get::<Option<String>, _>("message_id"),
            room_id: row.get::<i64, _>("room_id"),
            user_id: row.get::<i64, _>("user_id"),
            username: row.get::<String, _>("username"),
            message: row.get::<String, _>("message"),
            message_type: row.get::<String, _>("message_type"),
            is_emoji: row.get::<bool, _>("is_emoji"),
            created_at: row.get::<String, _>("created_at"),
            edited_at: row.get::<Option<String>, _>("edited_at"),
            deleted_at: row.get::<Option<String>, _>("deleted_at"),
        });
    }

    // Reverse to get chronological order
    messages.reverse();
    Ok(messages)
}

/// Edit a message's text, but only if `user_id` is the author and it isn't deleted.
/// Returns the number of rows affected (0 = not found / not authorized).
pub async fn edit_message_db(
    pool: &SqlitePool,
    message_id: &str,
    new_text: &str,
    user_id: i64,
) -> Result<u64, String> {
    let res = sqlx::query(
        "UPDATE messages
            SET message = $1, edited_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
          WHERE message_id = $2 AND user_id = $3 AND deleted_at IS NULL",
    )
    .bind(new_text)
    .bind(message_id)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to edit message: {}", e))?;
    Ok(res.rows_affected())
}

/// Soft-delete a message (clears text), only if `user_id` is the author.
pub async fn delete_message_db(
    pool: &SqlitePool,
    message_id: &str,
    user_id: i64,
) -> Result<u64, String> {
    let res = sqlx::query(
        "UPDATE messages
            SET message = '', deleted_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
          WHERE message_id = $1 AND user_id = $2 AND deleted_at IS NULL",
    )
    .bind(message_id)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to delete message: {}", e))?;
    Ok(res.rows_affected())
}

#[derive(Serialize)]
pub struct SearchResult {
    pub message_id: Option<String>,
    pub room_id: i64,
    pub room_name: String,
    pub username: String,
    pub message: String,
    pub created_at: String,
}

/// Full-text-ish search across non-deleted chat messages (case-insensitive LIKE).
#[tauri::command]
pub async fn search_messages(
    db: State<'_, SqlitePool>,
    query: String,
    limit: Option<i64>,
) -> Result<Vec<SearchResult>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.unwrap_or(50).clamp(1, 200);
    // Escape LIKE wildcards in the user query, then wrap with %...% via ESCAPE.
    let escaped = q
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = format!("%{}%", escaped);

    let rows = sqlx::query(
        "SELECT m.message_id, m.room_id, m.message, m.created_at,
                COALESCE(u.name, 'Unknown') AS username, cr.name AS room_name
         FROM messages m
         LEFT JOIN users u ON m.user_id = u.id
         JOIN chat_rooms cr ON m.room_id = cr.id
         WHERE m.deleted_at IS NULL
           AND m.message_type = 'Chat'
           AND m.message LIKE $1 ESCAPE '\\'
         ORDER BY m.created_at DESC, m.id DESC
         LIMIT $2",
    )
    .bind(&pattern)
    .bind(&limit)
    .fetch_all(&*db)
    .await
    .map_err(|e| format!("Search failed: {}", e))?;

    let mut results = Vec::new();
    for row in rows {
        results.push(SearchResult {
            message_id: row.get::<Option<String>, _>("message_id"),
            room_id: row.get::<i64, _>("room_id"),
            room_name: row.get::<String, _>("room_name"),
            username: row.get::<String, _>("username"),
            message: row.get::<String, _>("message"),
            created_at: row.get::<String, _>("created_at"),
        });
    }
    Ok(results)
}

#[derive(Serialize)]
pub struct ReactionAggregate {
    pub message_id: String,
    pub emoji: String,
    pub count: i64,
    pub me: bool,
}

/// Toggle one (message, user, emoji) reaction. Returns true if it was added, false if
/// it was removed.
pub async fn toggle_reaction_db(
    pool: &SqlitePool,
    message_id: &str,
    user_id: i64,
    emoji: &str,
) -> Result<bool, String> {
    let del =
        sqlx::query("DELETE FROM reactions WHERE message_id = $1 AND user_id = $2 AND emoji = $3")
            .bind(message_id)
            .bind(user_id)
            .bind(emoji)
            .execute(pool)
            .await
            .map_err(|e| format!("Failed to remove reaction: {}", e))?;

    if del.rows_affected() > 0 {
        return Ok(false); // removed
    }

    sqlx::query("INSERT INTO reactions (message_id, user_id, emoji) VALUES ($1, $2, $3)")
        .bind(message_id)
        .bind(user_id)
        .bind(emoji)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to add reaction: {}", e))?;
    Ok(true) // added
}

/// Aggregated reactions for every message in a room: per (message_id, emoji), the count
/// and whether `user_id` is among the reactors.
#[tauri::command]
pub async fn get_room_reactions(
    db: State<'_, SqlitePool>,
    room_id: i64,
    user_id: i64,
) -> Result<Vec<ReactionAggregate>, String> {
    let rows = sqlx::query(
        "SELECT r.message_id, r.emoji, COUNT(*) AS count,
                MAX(CASE WHEN r.user_id = $2 THEN 1 ELSE 0 END) AS me
         FROM reactions r
         JOIN messages m ON m.message_id = r.message_id
         WHERE m.room_id = $1
         GROUP BY r.message_id, r.emoji
         ORDER BY r.message_id",
    )
    .bind(&room_id)
    .bind(&user_id)
    .fetch_all(&*db)
    .await
    .map_err(|e| format!("Failed to load reactions: {}", e))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(ReactionAggregate {
            message_id: row.get::<String, _>("message_id"),
            emoji: row.get::<String, _>("emoji"),
            count: row.get::<i64, _>("count"),
            me: row.get::<i64, _>("me") != 0,
        });
    }
    Ok(out)
}
