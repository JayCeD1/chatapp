use tauri_plugin_sql::{Migration, MigrationKind};

pub fn get_migrations() -> Vec<Migration> {
    vec![
        // Migration 1: Create users table
        Migration {
            version: 1,
            description: "create_users_table",
            sql: "CREATE TABLE users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                email TEXT UNIQUE
            );",
            kind: MigrationKind::Up,
        },
        // Migration 2: Create posts table
        Migration {
            version: 2,
            description: "create_posts_table",
            sql: "CREATE TABLE posts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                title TEXT NOT NULL,
                content TEXT,
                FOREIGN KEY (user_id) REFERENCES users(id)
            );",
            kind: MigrationKind::Up,
        },
        // Migration 3: Create comments table
        Migration {
            version: 3,
            description: "create_comments_table",
            sql: "CREATE TABLE comments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                post_id INTEGER NOT NULL,
                user_id INTEGER NOT NULL,
                comment TEXT NOT NULL,
                FOREIGN KEY (post_id) REFERENCES posts(id),
                FOREIGN KEY (user_id) REFERENCES users(id)
            );",
            kind: MigrationKind::Up,
        },
    ]
}
