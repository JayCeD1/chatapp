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
