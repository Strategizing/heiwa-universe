import os
import psycopg2
from urllib.parse import urlparse

def fix_schema():
    db_url = os.getenv("DATABASE_URL")
    if not db_url:
        print("DATABASE_URL not found.")
        return

    print(f"Connecting to {db_url.split('@')[-1]}...")
    conn = psycopg2.connect(db_url)
    cursor = conn.cursor()
    
    columns = [
        ("node_id", "TEXT"),
        ("replay_receipt", "TEXT"),
        ("model_id", "TEXT"),
        ("tokens_input", "INTEGER"),
        ("tokens_output", "INTEGER"),
        ("tokens_total", "INTEGER"),
        ("cost", "REAL"),
        ("mode", "TEXT")
    ]
    
    for col_name, col_type in columns:
        try:
            cursor.execute(f"ALTER TABLE runs ADD COLUMN {col_name} {col_type}")
            print(f"✅ Added column {col_name}")
        except Exception as e:
            conn.rollback()
            print(f"ℹ️  Column {col_name} already exists or error: {e}")
            
    conn.commit()
    cursor.close()
    conn.close()
    print("✨ Schema Migration Complete.")

if __name__ == "__main__":
    fix_schema()
