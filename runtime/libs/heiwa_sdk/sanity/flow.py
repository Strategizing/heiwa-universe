import asyncio
import os

from libs.heiwa_sdk.routers.products import create_product, get_products
from libs.heiwa_sdk.schemas.product import ProductCreate
from libs.heiwa_sdk.db import db

async def run_verification():
    print("--- 🔧 MECHANIC V5: KINETIC VERIFICATION ---")

    # 1. Setup: Ensure table exists (since migration wasn't part of Gold Master)
    print("...checking database state...")
    conn = db.get_connection()
    try:
        cursor = conn.cursor()
        cursor.execute("""
            CREATE TABLE IF NOT EXISTS products (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                description TEXT,
                price REAL NOT NULL,
                in_stock BOOLEAN DEFAULT 1
            )
        """)
        conn.commit()
        print("...products table ready...")
    finally:
        conn.close()

    # 2. Initialize Dummy Object
    dummy_product = ProductCreate(
        name="Kinetic Widget",
        description="Verified in motion",
        price=19.99,
        in_stock=True,
        categories=["Testing"]
    )
    print(f"...prepared payload: {dummy_product.name}...")

    # 3. Test Creation
    print("...testing CREATE...")
    try:
        created = await create_product(dummy_product)
        # created is a dict because we return dict(zip(cols, row)) in router, 
        # but response_model=Product handles serialization in FastAPI. 
        # DIRECT CALL returns the dict directly as implementation writes it.
        # Wait, the router function `create_product` returns `dict(zip(cols, row))`.
        # So we expect a dict.
        print(f"✅ CREATE SUCCESS: ID={created.get('id')}, Name={created.get('name')}")
    except Exception as e:
        print(f"❌ CREATE FAILED: {e}")
        return

    # 4. Test Retrieval
    print("...testing READ...")
    try:
        products = await get_products()
        # products is list of dicts
        found = False
        for p in products:
            if p.get('id') == created.get('id'):
                found = True
                print(f"✅ READ SUCCESS: Found product {p.get('id')} in DB.")
                break
        
        if not found:
            print("❌ READ FAILED: Created product not found in list.")
            print(f"Current DB contents: {products}")
            return

    except Exception as e:
        print(f"❌ READ FAILED: {e}")
        return

    print("--- 🟢 SUCCESS: MECHANIC V5 VERIFIED ---")

def main():
    asyncio.run(run_verification())

if __name__ == "__main__":
    main()
