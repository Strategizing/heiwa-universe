from fastapi import APIRouter, HTTPException
from typing import List
from heiwa_sdk.schemas.product import Product, ProductCreate
from heiwa_sdk.db import db

router = APIRouter()

@router.get("/products", response_model=List[Product])
async def get_products():
    conn = db.get_connection()
    try:
        cursor = conn.cursor()
        cursor.execute("SELECT * FROM products")
        
        # Manual row to dict conversion
        cols = [column[0] for column in cursor.description]
        results = []
        for row in cursor.fetchall():
            results.append(dict(zip(cols, row)))
            
        return results
    finally:
        conn.close()

@router.post("/products", response_model=Product, status_code=201)
async def create_product(product: ProductCreate):
    conn = db.get_connection()
    try:
        cursor = conn.cursor()
        # Using ? placeholder for SQLite compatibility (default).
        query = """
            INSERT INTO products (name, description, price, in_stock)
            VALUES (?, ?, ?, ?)
        """
        
        cursor.execute(query, (product.name, product.description, product.price, product.in_stock))
        new_id = cursor.lastrowid
        conn.commit()
        
        # Fetch the created record
        cursor.execute("SELECT * FROM products WHERE rowid = ?", (new_id,))
        row = cursor.fetchone()
        
        if not row:
             raise HTTPException(status_code=500, detail="Failed to retrieve created product")

        cols = [column[0] for column in cursor.description]
        return dict(zip(cols, row))
        
    finally:
        conn.close()