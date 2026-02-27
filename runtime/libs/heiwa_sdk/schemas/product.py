from pydantic import BaseModel, Field, ConfigDict
from typing import Optional, List

# Create Base Model for shared fields
class ProductBase(BaseModel):
    name: str = Field(..., min_length=1, max_length=255, description="Name of the product")
    description: Optional[str] = Field(None, min_length=0, max_length=1000, description="Description of the product")
    price: float = Field(..., gt=0.0, description="Price of the product")
    in_stock: bool = Field(True, description="Whether the product is currently in stock")
    categories: Optional[List[str]] = Field(None, description="List of categories the product belongs to")

# Schema for creating a product (no ID)
class ProductCreate(ProductBase):
    pass

# Schema for reading a product (with ID)
class Product(ProductBase):
    id: int = Field(..., description="Unique identifier for the product")

    model_config = ConfigDict(
        validate_assignment=True,
        from_attributes=True,
        json_schema_extra={
            "example": {
                "id": 1,
                "name": "Example Product",
                "description": "This is an example product.",
                "price": 9.99,
                "in_stock": True,
                "categories": ["Electronics", "Gadgets"]
            }
        }
    )