//! Data-pack JSON crafting recipe loader and matcher.

use super::{SimpleSlot, is_empty};

/// Data-pack JSON recipe definition loaded from `<world_dir>/datapacks/recipes/*.json`.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DataPackRecipe {
    /// Human-readable recipe identifier.
    pub id: String,
    /// Required item IDs in the 3x3 grid (`0` = empty slot).
    pub ingredients: [i32; 9],
    /// Output item ID.
    pub result_item_id: i32,
    /// Output stack count.
    pub result_count: i32,
}

/// Loads all valid JSON recipe files from `<world_dir>/datapacks/recipes/`.
#[cfg_attr(not(test), allow(dead_code))]
#[must_use]
pub fn load_custom_recipes_from_dir(world_dir: &std::path::Path) -> Vec<DataPackRecipe> {
    let dir = world_dir.join("datapacks").join("recipes");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut recipes = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(raw) = std::fs::read_to_string(&path)
            && let Ok(recipe) = serde_json::from_str::<DataPackRecipe>(&raw)
            && recipe.result_item_id > 0
            && recipe.result_count > 0
        {
            recipes.push(recipe);
        }
    }
    recipes
}

/// Matches a 3x3 crafting grid against loaded data-pack recipes.
#[cfg_attr(not(test), allow(dead_code))]
#[must_use]
pub fn match_custom_recipe(grid: &[SimpleSlot; 9], recipes: &[DataPackRecipe]) -> SimpleSlot {
    for recipe in recipes {
        let matches = grid
            .iter()
            .zip(recipe.ingredients.iter())
            .all(|(slot, &req)| {
                if req == 0 {
                    is_empty(*slot)
                } else {
                    !is_empty(*slot) && slot.item_id == req
                }
            });
        if matches {
            return SimpleSlot {
                count: recipe.result_count,
                item_id: recipe.result_item_id,
            };
        }
    }
    SimpleSlot::default()
}
