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
    #[serde(default)]
    pub is_dm: bool,
    // For DMs, the display label derived from the *other* members (the stored `name` is a
    // synthetic key). None for regular channels — the UI falls back to `name`.
    #[serde(default)]
    pub display_name: Option<String>,
    pub user_count: Option<i64>,
}

// Build a ChatRoom from a query row. `is_dm`, `display_name`, `department_name` and
// `user_count` are optional columns — `try_get` yields the default when a query omits them.
fn row_to_room(row: &sqlx::sqlite::SqliteRow) -> ChatRoom {
    ChatRoom {
        id: row.get::<Option<i64>, _>("id"),
        name: row.get::<String, _>("name"),
        description: row.get::<Option<String>, _>("description"),
        department_id: row.get::<Option<i64>, _>("department_id"),
        department_name: row
            .try_get::<Option<String>, _>("department_name")
            .unwrap_or(None),
        is_private: row.get::<bool, _>("is_private"),
        is_dm: row.try_get::<bool, _>("is_dm").unwrap_or(false),
        display_name: row
            .try_get::<Option<String>, _>("display_name")
            .unwrap_or(None),
        user_count: row.try_get::<Option<i64>, _>("user_count").unwrap_or(None),
    }
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
    upsert_user_internal(&db, name, email, department_id).await
}

/// Pool-based upsert so the socket layer can register a connecting client into the HOST's
/// DB by email — making the host the single authority for user identity (globally-unique
/// ids), instead of trusting the per-instance id the client asserts.
pub async fn upsert_user_internal(
    pool: &SqlitePool,
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
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    {
        // Optionally update display name/department if changed
        sqlx::query("UPDATE users SET name=$1, department_id=$2 WHERE email=$3")
            .bind(&name)
            .bind(department_id)
            .bind(&email)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    } else {
        // Create
        sqlx::query("INSERT INTO users (name, email, department_id) VALUES ($1, $2, $3)")
            .bind(&name)
            .bind(&email)
            .bind(department_id)
            .execute(pool)
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
    .fetch_one(pool)
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
        .bind(department_id)
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
        .bind(id)
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
        .bind(is_online)
        .bind(user_id)
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
pub async fn get_chat_rooms(
    db: State<'_, SqlitePool>,
    user_id: i64,
) -> Result<Vec<ChatRoom>, String> {
    get_chat_rooms_internal(&db, user_id).await
}

/// Pool-based room listing so the socket layer can compute a client's authoritative room list
/// (public rooms + private rooms / DMs they belong to) on the HOST db and push it to them —
/// clients keep no usable local copy of host-created rooms.
pub async fn get_chat_rooms_internal(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<Vec<ChatRoom>, String> {
    let result = sqlx::query(
        "SELECT
  cr.id,
  cr.name,
  cr.description,
  cr.department_id,
  cr.is_private,
  cr.is_dm,
  d.name AS department_name,
  CASE WHEN cr.is_dm = 1 THEN (
    SELECT group_concat(u.name, ', ')
    FROM user_rooms ur2 JOIN users u ON u.id = ur2.user_id
    WHERE ur2.room_id = cr.id AND ur2.user_id != $1 AND ur2.is_active = 1
  ) ELSE NULL END AS display_name,
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
WHERE cr.is_private = 0
   OR cr.created_by = $1
   OR EXISTS (SELECT 1 FROM user_rooms ur
              WHERE ur.room_id = cr.id AND ur.user_id = $1 AND ur.is_active = 1)
ORDER BY cr.is_dm, cr.name
",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to get chat rooms: {}", e))?;

    let rooms = result.iter().map(row_to_room).collect();
    Ok(rooms)
}

#[tauri::command]
pub async fn get_rooms_by_department(
    db: State<'_, SqlitePool>,
    department_id: i64,
    user_id: i64,
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
  AND (cr.is_private = 0
       OR cr.created_by = $2
       OR EXISTS (SELECT 1 FROM user_rooms ur
                  WHERE ur.room_id = cr.id AND ur.user_id = $2 AND ur.is_active = 1))
ORDER BY cr.name
",
    )
    .bind(department_id)
    .bind(user_id)
    .fetch_all(&*db)
    .await
    .map_err(|e| format!("Failed to get rooms by department: {}", e))?;

    let rooms = result.iter().map(row_to_room).collect();
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
    .bind(department_id)
    .bind(is_private)
    .bind(created_by)
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

    // The creator is the first member — without this a private room would have no members
    // and be unjoinable (and unlistable) even by the person who made it.
    if let Some(creator) = created_by {
        sqlx::query(
            "INSERT INTO user_rooms (user_id, room_id, is_active) VALUES ($1, $2, 1)
             ON CONFLICT(user_id, room_id) DO UPDATE SET is_active = 1",
        )
        .bind(creator)
        .bind(id)
        .execute(&*db)
        .await
        .map_err(|e| format!("Failed to add creator to channel: {}", e))?;
    }

    let row = sqlx::query(
        "SELECT cr.id, cr.name, cr.description, cr.department_id, cr.is_private,
                d.name as department_name, 0 as user_count
         FROM chat_rooms cr LEFT JOIN departments d ON cr.department_id = d.id
         WHERE cr.id = $1",
    )
    .bind(id)
    .fetch_one(&*db)
    .await
    .map_err(|e| e.to_string())?;

    Ok(row_to_room(&row))
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
    .bind(user_id)
    .bind(room_id)
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
        .bind(user_id)
        .bind(room_id)
        .execute(&*db)
        .await
        .map_err(|e| format!("Failed to leave room: {}", e))?;

    Ok(())
}

/// Whether `user_id` may open `room_id`: the room is public, or the user created it, or the
/// user is an active member. Unknown room → not allowed. Used to enforce private channels.
pub async fn room_join_allowed_internal(
    pool: &SqlitePool,
    user_id: i64,
    room_id: i64,
) -> Result<bool, String> {
    let allowed: Option<bool> = sqlx::query_scalar(
        "SELECT (cr.is_private = 0
                 OR cr.created_by = $1
                 OR EXISTS (SELECT 1 FROM user_rooms ur
                            WHERE ur.room_id = cr.id AND ur.user_id = $1 AND ur.is_active = 1))
         FROM chat_rooms cr
         WHERE cr.id = $2",
    )
    .bind(user_id)
    .bind(room_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Failed to check room access: {}", e))?;
    Ok(allowed.unwrap_or(false))
}

/// Add `user_id` to `room_id` (an invite). Only someone who can already access the room
/// (its creator or an active member) may add others.
#[tauri::command]
pub async fn add_room_member(
    db: State<'_, SqlitePool>,
    room_id: i64,
    user_id: i64,
    actor_id: i64,
) -> Result<(), String> {
    add_room_member_internal(&db, room_id, user_id, actor_id).await
}

/// Pool-based variant so the socket layer can run client invites against the host DB.
pub async fn add_room_member_internal(
    pool: &SqlitePool,
    room_id: i64,
    user_id: i64,
    actor_id: i64,
) -> Result<(), String> {
    if !room_join_allowed_internal(pool, actor_id, room_id).await? {
        return Err("Only members can add people to this channel".to_string());
    }
    sqlx::query(
        "INSERT INTO user_rooms (user_id, room_id, is_active) VALUES ($1, $2, 1)
         ON CONFLICT(user_id, room_id) DO UPDATE SET is_active = 1",
    )
    .bind(user_id)
    .bind(room_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to add member: {}", e))?;
    Ok(())
}

/// Find or create a direct-message room between `actor_id` and `target_ids`.
///
/// A DM is a private room (`is_private = 1, is_dm = 1`) with a fixed member set. 1:1 DMs use a
/// deterministic name (`dm:<lo>:<hi>`) so repeat opens reuse the same room; group DMs (3+ people)
/// get a fresh `dm:g:<uuid>` each time. Returns the room from `actor_id`'s perspective, with
/// `display_name` set to the *other* members' names.
pub async fn get_or_create_dm_internal(
    pool: &SqlitePool,
    actor_id: i64,
    target_ids: Vec<i64>,
) -> Result<ChatRoom, String> {
    // Normalize to a sorted, de-duplicated member set including the actor.
    let mut members: Vec<i64> = target_ids;
    members.push(actor_id);
    members.sort_unstable();
    members.dedup();
    if members.len() < 2 {
        return Err("A direct message needs at least one other person".to_string());
    }
    if members.len() > 32 {
        return Err("A group message can have at most 32 people".to_string());
    }

    // Validate every member exists up front so we never create a half-populated DM room
    // (a missing target would otherwise fail the FK insert *after* the room row is created).
    for m in &members {
        let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM users WHERE id = $1")
            .bind(m)
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("Failed to validate DM member: {}", e))?;
        if exists.is_none() {
            return Err(format!("Unknown user {}", m));
        }
    }

    let is_group = members.len() > 2;
    let name = if is_group {
        format!("dm:g:{}", Uuid::new_v4())
    } else {
        format!("dm:{}:{}", members[0], members[1])
    };

    // 1:1 DMs are deduplicated by their deterministic name; group DMs are always new.
    let existing: Option<i64> = if is_group {
        None
    } else {
        sqlx::query_scalar("SELECT id FROM chat_rooms WHERE name = $1")
            .bind(&name)
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("Failed to look up direct message: {}", e))?
    };

    // Create the room and (re)activate every member atomically, so a mid-operation failure
    // can't leave an orphan room with a partial member set. Re-activation also covers reopening
    // a 1:1 a member had previously left, keeping the DM usable by the full set.
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("Failed to open transaction: {}", e))?;

    let room_id = match existing {
        Some(id) => id,
        None => {
            let res = sqlx::query(
                "INSERT INTO chat_rooms (name, description, is_private, is_dm, created_by)
                 VALUES ($1, NULL, 1, 1, $2)",
            )
            .bind(&name)
            .bind(actor_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("Failed to create direct message: {}", e))?;
            res.last_insert_rowid()
        }
    };

    for m in &members {
        sqlx::query(
            "INSERT INTO user_rooms (user_id, room_id, is_active) VALUES ($1, $2, 1)
             ON CONFLICT(user_id, room_id) DO UPDATE SET is_active = 1",
        )
        .bind(m)
        .bind(room_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Failed to add direct-message member: {}", e))?;
    }

    tx.commit()
        .await
        .map_err(|e| format!("Failed to commit direct message: {}", e))?;

    dm_room_view(pool, room_id, actor_id).await
}

/// Fetch a single room as a ChatRoom from `viewer_id`'s perspective (DM `display_name` =
/// the other members' names).
async fn dm_room_view(pool: &SqlitePool, room_id: i64, viewer_id: i64) -> Result<ChatRoom, String> {
    let row = sqlx::query(
        "SELECT cr.id, cr.name, cr.description, cr.department_id, cr.is_private, cr.is_dm,
                NULL AS department_name,
                CASE WHEN cr.is_dm = 1 THEN (
                  SELECT group_concat(u.name, ', ')
                  FROM user_rooms ur JOIN users u ON u.id = ur.user_id
                  WHERE ur.room_id = cr.id AND ur.user_id != $2 AND ur.is_active = 1
                ) ELSE NULL END AS display_name,
                (SELECT COUNT(DISTINCT user_id) FROM user_rooms
                 WHERE room_id = cr.id AND is_active = 1) AS user_count
         FROM chat_rooms cr WHERE cr.id = $1",
    )
    .bind(room_id)
    .bind(viewer_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Failed to load direct message: {}", e))?;
    Ok(row_to_room(&row))
}

/// A connectable user, for the client-side directory (invite + DM pickers). Clients keep no
/// local copy of the host's users, so the host pushes this list to them.
#[derive(Serialize)]
pub struct DirectoryUser {
    pub id: i64,
    pub name: String,
    pub is_online: bool,
}

pub async fn list_users_internal(pool: &SqlitePool) -> Result<Vec<DirectoryUser>, String> {
    let rows = sqlx::query("SELECT id, name, is_online FROM users ORDER BY name")
        .fetch_all(pool)
        .await
        .map_err(|e| format!("Failed to list users: {}", e))?;
    Ok(rows
        .into_iter()
        .map(|row| DirectoryUser {
            id: row.get::<i64, _>("id"),
            name: row.get::<String, _>("name"),
            is_online: row.get::<bool, _>("is_online"),
        })
        .collect())
}

#[tauri::command]
pub async fn list_users(db: State<'_, SqlitePool>) -> Result<Vec<DirectoryUser>, String> {
    list_users_internal(&db).await
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
    .bind(room_id)
    .bind(user_id)
    .bind(&message)
    .bind(&message_type)
    .bind(is_emoji)
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
    get_room_messages_internal(&db, room_id, limit.unwrap_or(50), before_id).await
}

/// Pool-based variant so the socket layer (host history sync) can reuse it.
/// before_id = None → newest `limit`. before_id = Some(id) → the `limit` messages
/// immediately older than `id`. Order + paginate by `id` (monotonic insertion order) so
/// the cursor and the sort key always agree.
pub async fn get_room_messages_internal(
    pool: &SqlitePool,
    room_id: i64,
    limit: i64,
    before_id: Option<i64>,
) -> Result<Vec<Message>, String> {
    let result = sqlx::query(
        "SELECT m.id, m.message_id, m.room_id, m.user_id, m.message, m.message_type, m.is_emoji, m.created_at,
                m.edited_at, m.deleted_at, COALESCE(u.name, 'Unknown') as username
         FROM messages m
         LEFT JOIN users u ON m.user_id = u.id
         WHERE m.room_id = $1 AND ($2 IS NULL OR m.id < $2)
         ORDER BY m.id DESC
         LIMIT $3",
    )
    .bind(room_id)
    .bind(before_id)
    .bind(limit)
    .fetch_all(pool)
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
    .bind(limit)
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
    // Run the DELETE-or-INSERT pair in one transaction so concurrent toggles of the same
    // (message, user, emoji) serialize (no count drift), and ON CONFLICT DO NOTHING so a
    // lost insert race can never surface a UNIQUE error.
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let del =
        sqlx::query("DELETE FROM reactions WHERE message_id = $1 AND user_id = $2 AND emoji = $3")
            .bind(message_id)
            .bind(user_id)
            .bind(emoji)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("Failed to remove reaction: {}", e))?;

    let added = if del.rows_affected() > 0 {
        false // removed
    } else {
        sqlx::query(
            "INSERT INTO reactions (message_id, user_id, emoji) VALUES ($1, $2, $3)
             ON CONFLICT(message_id, user_id, emoji) DO NOTHING",
        )
        .bind(message_id)
        .bind(user_id)
        .bind(emoji)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Failed to add reaction: {}", e))?;
        true // added
    };

    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(added)
}

/// Aggregated reactions for every message in a room: per (message_id, emoji), the count
/// and whether `user_id` is among the reactors.
#[tauri::command]
pub async fn get_room_reactions(
    db: State<'_, SqlitePool>,
    room_id: i64,
    user_id: i64,
) -> Result<Vec<ReactionAggregate>, String> {
    get_room_reactions_internal(&db, room_id, user_id).await
}

/// Pool-based variant for the socket layer (host history sync).
pub async fn get_room_reactions_internal(
    pool: &SqlitePool,
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
    .bind(room_id)
    .bind(user_id)
    .fetch_all(pool)
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

#[derive(Serialize)]
pub struct UnreadCount {
    pub room_id: i64,
    pub count: i64,
}

/// Mark a room read for a user: set `last_read_at` to now (CURRENT_TIMESTAMP format, so it
/// compares directly with `messages.created_at`). Upserts the `user_rooms` row so it works
/// even before an explicit join.
pub async fn touch_last_read_internal(
    pool: &SqlitePool,
    user_id: i64,
    room_id: i64,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO user_rooms (user_id, room_id, last_read_at, is_active)
         VALUES ($1, $2, datetime('now'), 1)
         ON CONFLICT(user_id, room_id)
            DO UPDATE SET last_read_at = datetime('now'), is_active = 1",
    )
    .bind(user_id)
    .bind(room_id)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to mark room read: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn touch_last_read(
    db: State<'_, SqlitePool>,
    user_id: i64,
    room_id: i64,
) -> Result<(), String> {
    touch_last_read_internal(&db, user_id, room_id).await
}

/// Per-room unread counts for a user: chat messages (not system events, not deleted, not
/// the user's own) newer than the room's `last_read_at`. Only rooms the user belongs to
/// (has a `user_rooms` row for) and that have at least one unread are returned.
pub async fn get_unread_counts_internal(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<Vec<UnreadCount>, String> {
    let rows = sqlx::query(
        "SELECT m.room_id AS room_id, COUNT(*) AS count
         FROM messages m
         JOIN user_rooms ur ON ur.room_id = m.room_id AND ur.user_id = $1
         WHERE m.user_id != $1
           AND m.message_type = 'Chat'
           AND m.deleted_at IS NULL
           AND (ur.last_read_at IS NULL OR m.created_at > ur.last_read_at)
         GROUP BY m.room_id
         HAVING COUNT(*) > 0",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to get unread counts: {}", e))?;

    Ok(rows
        .into_iter()
        .map(|row| UnreadCount {
            room_id: row.get::<i64, _>("room_id"),
            count: row.get::<i64, _>("count"),
        })
        .collect())
}

#[tauri::command]
pub async fn get_unread_counts(
    db: State<'_, SqlitePool>,
    user_id: i64,
) -> Result<Vec<UnreadCount>, String> {
    get_unread_counts_internal(&db, user_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use tauri_plugin_sql::MigrationKind;

    // A single-connection in-memory DB with the REAL migrations applied. raw_sql is used
    // so multi-statement migrations (e.g. the index/rebuild ones) run in full. Migrations
    // 6/7 already seed departments and a room with id 1, so we only add the two users
    // (Alice=1, Bob=2) the tests act as; messages go into the pre-seeded room 1.
    async fn setup() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory db");

        for m in crate::migration::get_migrations() {
            if matches!(m.kind, MigrationKind::Up) {
                sqlx::raw_sql(m.sql)
                    .execute(&pool)
                    .await
                    .unwrap_or_else(|e| panic!("migration {} failed: {e}", m.version));
            }
        }

        sqlx::raw_sql(
            "INSERT INTO users (id, name, email, department_id)
                 VALUES (1, 'Alice', 'a@x', 1), (2, 'Bob', 'b@x', 1);",
        )
        .execute(&pool)
        .await
        .expect("seed users");

        pool
    }

    async fn add(pool: &SqlitePool, user: i64, text: &str, mid: &str) {
        save_message_internal(pool, 1, user, text.into(), "Chat".into(), false, mid.into())
            .await
            .expect("save message");
    }

    // Insert a message with an EXPLICIT created_at + type so unread tests don't depend on
    // wall-clock / same-second timing.
    async fn insert_at(pool: &SqlitePool, user: i64, mtype: &str, created_at: &str, mid: &str) {
        sqlx::query(
            "INSERT INTO messages (room_id, user_id, message, message_type, is_emoji, message_id, created_at)
             VALUES (1, $1, 'x', $2, 0, $3, $4)",
        )
        .bind(user)
        .bind(mtype)
        .bind(mid)
        .bind(created_at)
        .execute(pool)
        .await
        .expect("insert message");
    }

    #[tokio::test]
    async fn unread_counts_respect_last_read_excluding_own_and_system() {
        let pool = setup().await;
        // Alice (1) is a member of room 1, last read at a fixed past time.
        touch_last_read_internal(&pool, 1, 1).await.unwrap();
        sqlx::raw_sql(
            "UPDATE user_rooms SET last_read_at = '2026-01-01 00:00:00' WHERE user_id=1 AND room_id=1",
        )
        .execute(&pool)
        .await
        .unwrap();

        insert_at(&pool, 2, "Chat", "2026-02-01 00:00:00", "b1").await; // unread
        insert_at(&pool, 2, "Chat", "2026-02-01 00:00:01", "b2").await; // unread
        insert_at(&pool, 2, "RoomJoin", "2026-02-01 00:00:02", "sys").await; // system, ignored
        insert_at(&pool, 1, "Chat", "2026-02-01 00:00:03", "own").await; // own, ignored
        insert_at(&pool, 2, "Chat", "2025-12-01 00:00:00", "old").await; // before read, ignored

        let counts = get_unread_counts_internal(&pool, 1).await.unwrap();
        assert_eq!(counts.len(), 1);
        assert_eq!(counts[0].room_id, 1);
        assert_eq!(counts[0].count, 2);

        // Marking the room read clears it.
        sqlx::raw_sql(
            "UPDATE user_rooms SET last_read_at = '2026-03-01 00:00:00' WHERE user_id=1 AND room_id=1",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(get_unread_counts_internal(&pool, 1)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn touch_last_read_upserts_and_sets_marker() {
        let pool = setup().await;
        touch_last_read_internal(&pool, 1, 1).await.unwrap();
        let read_at: Option<String> =
            sqlx::query_scalar("SELECT last_read_at FROM user_rooms WHERE user_id=1 AND room_id=1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(read_at.is_some(), "last_read_at should be set after touch");
    }

    #[tokio::test]
    async fn pagination_orders_chronologically_and_cursors_backwards() {
        let pool = setup().await;
        add(&pool, 1, "one", "m1").await;
        add(&pool, 1, "two", "m2").await;
        add(&pool, 2, "three", "m3").await;

        let all = get_room_messages_internal(&pool, 1, 50, None)
            .await
            .unwrap();
        let texts: Vec<_> = all.iter().map(|m| m.message.as_str()).collect();
        assert_eq!(texts, ["one", "two", "three"]); // oldest → newest

        // Newest page of 2.
        let page = get_room_messages_internal(&pool, 1, 2, None).await.unwrap();
        let texts: Vec<_> = page.iter().map(|m| m.message.as_str()).collect();
        assert_eq!(texts, ["two", "three"]);

        // Everything strictly older than id 2 — the cursor and sort key agree.
        let older = get_room_messages_internal(&pool, 1, 50, Some(2))
            .await
            .unwrap();
        let texts: Vec<_> = older.iter().map(|m| m.message.as_str()).collect();
        assert_eq!(texts, ["one"]);
    }

    #[tokio::test]
    async fn edit_is_author_scoped_and_sets_edited_at() {
        let pool = setup().await;
        add(&pool, 1, "hi", "m1").await;

        // Bob cannot edit Alice's message; Alice can.
        assert_eq!(edit_message_db(&pool, "m1", "hacked", 2).await.unwrap(), 0);
        assert_eq!(edit_message_db(&pool, "m1", "fixed", 1).await.unwrap(), 1);

        let msgs = get_room_messages_internal(&pool, 1, 50, None)
            .await
            .unwrap();
        assert_eq!(msgs[0].message, "fixed");
        assert!(msgs[0].edited_at.is_some());
    }

    #[tokio::test]
    async fn delete_is_author_scoped_and_blocks_later_edit() {
        let pool = setup().await;
        add(&pool, 1, "secret", "m1").await;

        assert_eq!(delete_message_db(&pool, "m1", 2).await.unwrap(), 0); // Bob can't
        assert_eq!(delete_message_db(&pool, "m1", 1).await.unwrap(), 1); // Alice can
                                                                         // Editing a deleted message is a no-op (the `deleted_at IS NULL` guard).
        assert_eq!(edit_message_db(&pool, "m1", "back", 1).await.unwrap(), 0);

        let msgs = get_room_messages_internal(&pool, 1, 50, None)
            .await
            .unwrap();
        assert_eq!(msgs[0].message, "");
        assert!(msgs[0].deleted_at.is_some());
    }

    #[tokio::test]
    async fn reaction_toggle_updates_count_and_me_flag() {
        let pool = setup().await;
        add(&pool, 1, "react to me", "m1").await;

        assert!(toggle_reaction_db(&pool, "m1", 1, "👍").await.unwrap()); // Alice adds
        assert!(toggle_reaction_db(&pool, "m1", 2, "👍").await.unwrap()); // Bob adds

        let agg = get_room_reactions_internal(&pool, 1, 1).await.unwrap();
        assert_eq!(agg.len(), 1);
        assert_eq!(agg[0].emoji, "👍");
        assert_eq!(agg[0].count, 2);
        assert!(agg[0].me); // Alice is among the reactors

        // Alice removes hers → count drops, her `me` flips false.
        assert!(!toggle_reaction_db(&pool, "m1", 1, "👍").await.unwrap());
        let agg = get_room_reactions_internal(&pool, 1, 1).await.unwrap();
        assert_eq!(agg[0].count, 1);
        assert!(!agg[0].me);
        // ...but Bob still sees it as his own.
        let agg_bob = get_room_reactions_internal(&pool, 1, 2).await.unwrap();
        assert!(agg_bob[0].me);
    }

    #[tokio::test]
    async fn private_room_access_is_member_scoped() {
        let pool = setup().await;
        // Public room (room 1 is pre-seeded, is_private=0): anyone is allowed.
        assert!(room_join_allowed_internal(&pool, 2, 1).await.unwrap());

        // A private room owned by Alice (1).
        sqlx::raw_sql(
            "INSERT INTO chat_rooms (id, name, is_private, created_by) VALUES (100, 'secret', 1, 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(room_join_allowed_internal(&pool, 1, 100).await.unwrap()); // creator
        assert!(!room_join_allowed_internal(&pool, 2, 100).await.unwrap()); // non-member denied

        // Add Bob (2) as a member → now allowed.
        sqlx::raw_sql("INSERT INTO user_rooms (user_id, room_id, is_active) VALUES (2, 100, 1)")
            .execute(&pool)
            .await
            .unwrap();
        assert!(room_join_allowed_internal(&pool, 2, 100).await.unwrap());

        // Leaving (is_active=0) revokes access; unknown room is denied.
        sqlx::raw_sql("UPDATE user_rooms SET is_active = 0 WHERE user_id = 2 AND room_id = 100")
            .execute(&pool)
            .await
            .unwrap();
        assert!(!room_join_allowed_internal(&pool, 2, 100).await.unwrap());
        assert!(!room_join_allowed_internal(&pool, 1, 999).await.unwrap());
    }

    #[tokio::test]
    async fn save_is_idempotent_on_message_id() {
        let pool = setup().await;
        add(&pool, 1, "once", "dup").await;

        // Same message_id again — ON CONFLICT(message_id) DO NOTHING.
        let r = save_message_internal(
            &pool,
            1,
            1,
            "twice".into(),
            "Chat".into(),
            false,
            "dup".into(),
        )
        .await
        .unwrap();
        assert_eq!(r.rows_affected, 0);

        let all = get_room_messages_internal(&pool, 1, 50, None)
            .await
            .unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].message, "once");
    }

    #[tokio::test]
    async fn dm_one_to_one_is_found_or_created_and_labeled_by_other_member() {
        let pool = setup().await;

        // Alice (1) opens a DM with Bob (2).
        let dm = get_or_create_dm_internal(&pool, 1, vec![2]).await.unwrap();
        assert!(dm.is_dm);
        assert!(dm.is_private);
        assert_eq!(dm.name, "dm:1:2"); // deterministic, sorted
                                       // From Alice's perspective the label is the *other* member.
        assert_eq!(dm.display_name.as_deref(), Some("Bob"));

        // Opening again (in either order) reuses the same room — no duplicate.
        let again = get_or_create_dm_internal(&pool, 2, vec![1]).await.unwrap();
        assert_eq!(again.id, dm.id);
        assert_eq!(again.display_name.as_deref(), Some("Alice")); // Bob's perspective
    }

    #[tokio::test]
    async fn dm_group_is_always_new_and_validates_members() {
        let pool = setup().await;
        sqlx::raw_sql("INSERT INTO users (id, name, email) VALUES (3, 'Carol', 'c@x')")
            .execute(&pool)
            .await
            .unwrap();

        // A 3-person group DM gets a fresh room each time (no dedup).
        let g1 = get_or_create_dm_internal(&pool, 1, vec![2, 3])
            .await
            .unwrap();
        let g2 = get_or_create_dm_internal(&pool, 1, vec![2, 3])
            .await
            .unwrap();
        assert!(g1.is_dm && g2.is_dm);
        assert_ne!(g1.id, g2.id);
        assert!(g1.name.starts_with("dm:g:"));

        // A non-existent target is rejected before any room row is created.
        let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chat_rooms")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(get_or_create_dm_internal(&pool, 1, vec![999])
            .await
            .is_err());
        let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chat_rooms")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(before, after); // no half-created DM room

        // A DM with only yourself is rejected.
        assert!(get_or_create_dm_internal(&pool, 1, vec![1]).await.is_err());
    }

    #[tokio::test]
    async fn dm_rooms_are_listed_only_for_members() {
        let pool = setup().await;
        sqlx::raw_sql("INSERT INTO users (id, name, email) VALUES (3, 'Carol', 'c@x')")
            .execute(&pool)
            .await
            .unwrap();
        let dm = get_or_create_dm_internal(&pool, 1, vec![2]).await.unwrap();

        // Members see the DM (labeled by the other member); a non-member (Carol) does not.
        let allowed = room_join_allowed_internal(&pool, 1, dm.id.unwrap())
            .await
            .unwrap();
        let denied = room_join_allowed_internal(&pool, 3, dm.id.unwrap())
            .await
            .unwrap();
        assert!(allowed);
        assert!(!denied);
    }
}
