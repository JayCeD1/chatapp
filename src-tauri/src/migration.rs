use tauri_plugin_sql::{Migration, MigrationKind};

pub fn get_migrations() -> Vec<Migration> {
    vec![
        // Migration 1: Create departments table
        Migration {
            version: 1,
            description: "create_departments_table",
            sql: "CREATE TABLE departments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                description TEXT,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );",
            kind: MigrationKind::Up,
        },
        // Migration 2: Create users table
        Migration {
            version: 2,
            description: "create_users_table",
            sql: "CREATE TABLE users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                email TEXT UNIQUE,
                department_id INTEGER,
                is_online BOOLEAN DEFAULT FALSE,
                last_seen TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (department_id) REFERENCES departments(id)
            );",
            kind: MigrationKind::Up,
        },
        // Migration 3: Create chat rooms table
        Migration {
            version: 3,
            description: "create_chat_rooms_table",
            sql: "CREATE TABLE chat_rooms (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                description TEXT,
                department_id INTEGER,
                is_private BOOLEAN DEFAULT FALSE,
                created_by INTEGER,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (department_id) REFERENCES departments(id),
                FOREIGN KEY (created_by) REFERENCES users(id)
            );",
            kind: MigrationKind::Up,
        },
        // Migration 4: Create user_rooms table (many-to-many relationship)
        Migration {
            version: 4,
            description: "create_user_rooms_table",
            sql: "CREATE TABLE user_rooms (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                room_id INTEGER NOT NULL,
                joined_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                is_active BOOLEAN DEFAULT TRUE,
                FOREIGN KEY (user_id) REFERENCES users(id),
                FOREIGN KEY (room_id) REFERENCES chat_rooms(id),
                UNIQUE(user_id, room_id)
            );",
            kind: MigrationKind::Up,
        },
        // Migration 5: Create messages table
        Migration {
            version: 5,
            description: "create_messages_table",
            sql: "CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                room_id INTEGER NOT NULL,
                user_id INTEGER NOT NULL,
                message TEXT NOT NULL,
                message_type TEXT DEFAULT 'chat',
                is_emoji BOOLEAN DEFAULT FALSE,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (room_id) REFERENCES chat_rooms(id),
                FOREIGN KEY (user_id) REFERENCES users(id)
            );",
            kind: MigrationKind::Up,
        },
        // Migration 6: Insert default departments
        Migration {
            version: 6,
            description: "insert_default_departments",
            sql: "INSERT INTO departments (name, description) VALUES 
                   ('IT', 'Information Technology Department'),
                   ('HR', 'Human Resources Department'),
                   ('Finance', 'Finance and Accounting Department'),
                   ('Marketing', 'Marketing and Sales Department'),
                   ('Operations', 'Operations Department'),
                   ('General', 'General Company Chat');",
            kind: MigrationKind::Up,
        },
        // Migration 7: Create default chat rooms for each department
        Migration {
            version: 7,
            description: "create_default_chat_rooms",
            sql: "INSERT INTO chat_rooms (name, description, department_id, is_private) 
                   SELECT 
                       d.name || ' General' as name,
                       'General chat room for ' || d.name || ' department' as description,
                       d.id as department_id,
                       FALSE as is_private
                   FROM departments d
                   WHERE d.name != 'General';
                   
                   INSERT INTO chat_rooms (name, description, department_id, is_private) 
                   VALUES ('Company Wide', 'General company chat room', 
                          (SELECT id FROM departments WHERE name = 'General'), FALSE);",
            kind: MigrationKind::Up,
        },
        // Migration 8: message_id for reliable dedup/idempotency, perf indexes,
        // and a UNIQUE name so name-keyed broadcast routing can't collapse two rooms.
        Migration {
            version: 8,
            description: "add_message_id_and_indexes",
            sql: "ALTER TABLE messages ADD COLUMN message_id TEXT;
                  CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_message_id ON messages(message_id);
                  CREATE INDEX IF NOT EXISTS idx_messages_room_created ON messages(room_id, created_at, id);
                  CREATE INDEX IF NOT EXISTS idx_messages_user ON messages(user_id);
                  CREATE INDEX IF NOT EXISTS idx_user_rooms_room_active ON user_rooms(room_id, is_active);
                  CREATE UNIQUE INDEX IF NOT EXISTS idx_chat_rooms_name ON chat_rooms(name);",
            kind: MigrationKind::Up,
        },
        // Down for v8
        Migration {
            version: 8,
            description: "drop_message_id_and_indexes",
            sql: "DROP INDEX IF EXISTS idx_chat_rooms_name;
                  DROP INDEX IF EXISTS idx_user_rooms_room_active;
                  DROP INDEX IF EXISTS idx_messages_user;
                  DROP INDEX IF EXISTS idx_messages_room_created;
                  DROP INDEX IF EXISTS idx_messages_message_id;",
            kind: MigrationKind::Down,
        },
        // Migration 9: rebuild the child tables (messages, user_rooms) with ON DELETE
        // CASCADE so deleting a room/user removes its messages + memberships instead of
        // leaving orphans. Child tables only (nothing references them), so this is safe
        // with foreign_keys=ON; a defensive orphan-cleanup guarantees the FK-checked
        // re-insert can't fail. Indexes are recreated afterward.
        Migration {
            version: 9,
            description: "add_on_delete_cascade_to_child_tables",
            sql: "DELETE FROM messages
                      WHERE room_id NOT IN (SELECT id FROM chat_rooms)
                         OR user_id NOT IN (SELECT id FROM users);
                  CREATE TABLE messages_new (
                      id INTEGER PRIMARY KEY AUTOINCREMENT,
                      room_id INTEGER NOT NULL,
                      user_id INTEGER NOT NULL,
                      message TEXT NOT NULL,
                      message_type TEXT DEFAULT 'chat',
                      is_emoji BOOLEAN DEFAULT FALSE,
                      created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                      message_id TEXT,
                      FOREIGN KEY (room_id) REFERENCES chat_rooms(id) ON DELETE CASCADE,
                      FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
                  );
                  INSERT INTO messages_new (id, room_id, user_id, message, message_type, is_emoji, created_at, message_id)
                      SELECT id, room_id, user_id, message, message_type, is_emoji, created_at, message_id FROM messages;
                  DROP TABLE messages;
                  ALTER TABLE messages_new RENAME TO messages;
                  CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_message_id ON messages(message_id);
                  CREATE INDEX IF NOT EXISTS idx_messages_room_created ON messages(room_id, created_at, id);
                  CREATE INDEX IF NOT EXISTS idx_messages_user ON messages(user_id);

                  DELETE FROM user_rooms
                      WHERE user_id NOT IN (SELECT id FROM users)
                         OR room_id NOT IN (SELECT id FROM chat_rooms);
                  CREATE TABLE user_rooms_new (
                      id INTEGER PRIMARY KEY AUTOINCREMENT,
                      user_id INTEGER NOT NULL,
                      room_id INTEGER NOT NULL,
                      joined_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                      is_active BOOLEAN DEFAULT TRUE,
                      FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
                      FOREIGN KEY (room_id) REFERENCES chat_rooms(id) ON DELETE CASCADE,
                      UNIQUE(user_id, room_id)
                  );
                  INSERT INTO user_rooms_new (id, user_id, room_id, joined_at, is_active)
                      SELECT id, user_id, room_id, joined_at, is_active FROM user_rooms;
                  DROP TABLE user_rooms;
                  ALTER TABLE user_rooms_new RENAME TO user_rooms;
                  CREATE INDEX IF NOT EXISTS idx_user_rooms_room_active ON user_rooms(room_id, is_active);",
            kind: MigrationKind::Up,
        },
        // Migration 10: message edit/delete metadata.
        Migration {
            version: 10,
            description: "add_message_edit_delete_columns",
            sql: "ALTER TABLE messages ADD COLUMN edited_at TEXT;
                  ALTER TABLE messages ADD COLUMN deleted_at TEXT;",
            kind: MigrationKind::Up,
        },
        // Migration 11: emoji reactions (keyed by the message UUID).
        Migration {
            version: 11,
            description: "create_reactions_table",
            sql: "CREATE TABLE reactions (
                      id INTEGER PRIMARY KEY AUTOINCREMENT,
                      message_id TEXT NOT NULL,
                      user_id INTEGER NOT NULL,
                      emoji TEXT NOT NULL,
                      created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                      UNIQUE(message_id, user_id, emoji),
                      FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
                  );
                  CREATE INDEX idx_reactions_message ON reactions(message_id);",
            kind: MigrationKind::Up,
        },
        // Migration 12: per-(user, room) read marker for unread tracking. NULL = never
        // read (everything counts as unread). Stored in CURRENT_TIMESTAMP format (via
        // datetime('now')) so it compares directly with messages.created_at.
        Migration {
            version: 12,
            description: "add_user_rooms_last_read_at",
            sql: "ALTER TABLE user_rooms ADD COLUMN last_read_at TIMESTAMP;",
            kind: MigrationKind::Up,
        },
        // Migration 13: mark direct-message rooms. A DM is a private room (is_private = 1)
        // with is_dm = 1; its stored name is a synthetic key (dm:<a>:<b> for 1:1, dm:g:<uuid>
        // for groups), while the UI shows a name derived from the other members.
        Migration {
            version: 13,
            description: "add_chat_rooms_is_dm",
            sql: "ALTER TABLE chat_rooms ADD COLUMN is_dm BOOLEAN NOT NULL DEFAULT 0;",
            kind: MigrationKind::Up,
        },
        // Down for v7: remove default chat rooms created in v7
        Migration {
            version: 7,
            description: "drop_default_chat_rooms",
            sql: "
                DELETE FROM chat_rooms WHERE name = 'Company Wide';
                DELETE FROM chat_rooms
                  WHERE name LIKE '% General';
            ",
            kind: MigrationKind::Down,
        },
        // Down for v6: remove the seeded departments from v6
        Migration {
            version: 6,
            description: "remove_default_departments",
            sql: "
                DELETE FROM departments
                  WHERE name IN ('IT','HR','Finance','Marketing','Operations','General');
            ",
            kind: MigrationKind::Down,
        },
        // Down for v5: drop messages table
        Migration {
            version: 5,
            description: "drop_messages_table",
            sql: "DROP TABLE IF EXISTS messages;",
            kind: MigrationKind::Down,
        },
        // Down for v4: drop user_rooms table
        Migration {
            version: 4,
            description: "drop_user_rooms_table",
            sql: "DROP TABLE IF EXISTS user_rooms;",
            kind: MigrationKind::Down,
        },
        // Down for v3: drop chat_rooms table
        Migration {
            version: 3,
            description: "drop_chat_rooms_table",
            sql: "DROP TABLE IF EXISTS chat_rooms;",
            kind: MigrationKind::Down,
        },
        // Down for v2: drop users table
        Migration {
            version: 2,
            description: "drop_users_table",
            sql: "DROP TABLE IF EXISTS users;",
            kind: MigrationKind::Down,
        },
        // Down for v1: drop departments table
        Migration {
            version: 1,
            description: "drop_departments_table",
            sql: "DROP TABLE IF EXISTS departments;",
            kind: MigrationKind::Down,
        },

    ]
}
