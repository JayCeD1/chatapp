use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::State;

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
    pub room_id: i64,
    pub user_id: i64,
    pub username: String,
    pub message: String,
    pub message_type: String,
    pub is_emoji: bool,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct InsertResult {
    pub rows_affected: u64,
    pub last_insert_id: i64,
}

// User management
#[tauri::command]
pub async fn upsert_user(
    db:State<'_,SqlitePool>,
    name: String,
    email: String,
    department_id: Option<i64>,
) -> Result<User, String> {
    //Try find existing
    if let Some (row) = sqlx::query(
        "SELECT u.id, u.name, u.email, u.department_id, u.is_online, u.last_seen,
                d.name as department_name
         FROM users u LEFT JOIN departments d ON u.department_id = d.id
         WHERE u.email = $1"
    ).bind(&email)
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
         WHERE u.email = $1"
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
        "SELECT cr.id, cr.name, cr.description, cr.department_id, cr.is_private, 
                d.name as department_name,
                COUNT(ur.user_id) as user_count
         FROM chat_rooms cr
         LEFT JOIN departments d ON cr.department_id = d.id
         LEFT JOIN user_rooms ur ON cr.id = ur.room_id AND ur.is_active = 1
         GROUP BY cr.id
         ORDER BY cr.name"
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
        "SELECT cr.id, cr.name, cr.description, cr.department_id, cr.is_private, 
                d.name as department_name,
                COUNT(ur.user_id) as user_count
         FROM chat_rooms cr
         LEFT JOIN departments d ON cr.department_id = d.id
         LEFT JOIN user_rooms ur ON cr.id = ur.room_id AND ur.is_active = 1
         WHERE cr.department_id = $1
         GROUP BY cr.id
         ORDER BY cr.name"
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
pub async fn join_room(
    db: State<'_, SqlitePool>,
    user_id: i64,
    room_id: i64,
) -> Result<(), String> {
    sqlx::query(
        "INSERT OR REPLACE INTO user_rooms (user_id, room_id, is_active) VALUES ($1, $2, 1)"
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
    let result = sqlx::query(
        "INSERT INTO messages (room_id, user_id, message, message_type, is_emoji) 
         VALUES ($1, $2, $3, $4, $5)"
    )
        .bind(&room_id)
        .bind(&user_id)
        .bind(&message)
        .bind(&message_type)
        .bind(&is_emoji)
        .execute(&*db)
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
) -> Result<Vec<Message>, String> {
    let limit = limit.unwrap_or(50);
    
    let result = sqlx::query(
        "SELECT m.id, m.room_id, m.user_id, m.message, m.message_type, m.is_emoji, m.created_at,
                u.name as username
         FROM messages m
         JOIN users u ON m.user_id = u.id
         WHERE m.room_id = $1
         ORDER BY m.created_at DESC
         LIMIT $2"
    )
        .bind(&room_id)
        .bind(&limit)
        .fetch_all(&*db)
        .await
        .map_err(|e| format!("Failed to get room messages: {}", e))?;

    let mut messages = Vec::new();
    for row in result {
        messages.push(Message {
            id: row.get::<Option<i64>, _>("id"),
            room_id: row.get::<i64, _>("room_id"),
            user_id: row.get::<i64, _>("user_id"),
            username: row.get::<String, _>("username"),
            message: row.get::<String, _>("message"),
            message_type: row.get::<String, _>("message_type"),
            is_emoji: row.get::<bool, _>("is_emoji"),
            created_at: row.get::<String, _>("created_at"),
        });
    }
    
    // Reverse to get chronological order
    messages.reverse();
    Ok(messages)
}