#!/usr/bin/env python3
import os
import sys
import psycopg2

def init_db():
    database_url = os.getenv("DATABASE_URL")
    if not database_url:
        print("❌ Error: DATABASE_URL environment variable is not set.")
        sys.exit(1)

    print(f"🔌 Connecting to database...")
    
    try:
        conn = psycopg2.connect(database_url)
        cur = conn.cursor()
        
        print("🛠️  Creating 'tasks' table if not exists...")
        cur.execute("""
            CREATE TABLE IF NOT EXISTS tasks (
                id SERIAL PRIMARY KEY,
                discord_msg_id VARCHAR(50),
                source VARCHAR(50),
                payload TEXT NOT NULL,
                status VARCHAR(20) DEFAULT 'pending', -- pending, processing, completed, failed
                result TEXT,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );
        """)
        
        print("⚡ Creating index on 'status'...")
        cur.execute("""
            CREATE INDEX IF NOT EXISTS idx_status ON tasks(status);
        """)
        
        conn.commit()
        cur.close()
        conn.close()
        print("✅ Database initialized successfully.")
        
    except Exception as e:
        print(f"❌ Database Error: {e}")
        sys.exit(1)

if __name__ == "__main__":
    init_db()
