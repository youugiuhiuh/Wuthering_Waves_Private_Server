```markdown
# Wuthering_Waves_Private_Server Development Patterns

> Auto-generated skill from repository analysis

## Overview
This skill teaches you the core development patterns and workflows used in the `Wuthering_Waves_Private_Server` Rust codebase. You'll learn about the project's coding conventions, how to implement and register new logic modules, and how to follow the repository's commit and testing practices.

## Coding Conventions

### File Naming
- **Style:** camelCase
- **Example:**  
  ```
  playerLogic.rs
  enemyAI.rs
  ```

### Import Style
- **Style:** Relative imports
- **Example:**
  ```rust
  mod playerLogic;
  use crate::logic::playerLogic::Player;
  ```

### Export Style
- **Style:** Named exports
- **Example:**
  ```rust
  pub struct Player { /* ... */ }
  pub fn handle_action() { /* ... */ }
  ```

### Commit Messages
- **Type:** Conventional commits
- **Prefixes:** `feat`, `refactor`
- **Example:**
  ```
  feat: add player movement logic
  refactor: optimize enemy AI pathfinding
  ```

## Workflows

### Feature Module Implementation & Update Mod
**Trigger:** When you want to add a new logic module to the project  
**Command:** `/new-logic-module`

1. **Create or update the feature module file**  
   - Path: `rust/tgbot/src/logic/<feature>.rs`
   - Example:
     ```rust
     // rust/tgbot/src/logic/playerMovement.rs
     pub fn move_player(/* params */) {
         // Implementation
     }
     ```
2. **Register the new module in the module registry**  
   - Edit: `rust/tgbot/src/logic/mod.rs`
   - Example:
     ```rust
     pub mod playerMovement;
     // Add further module registrations here
     ```

## Testing Patterns

- **Framework:** Unknown (no explicit framework detected)
- **File Pattern:** `*.test.*`
- **Example:**
  ```
  rust/tgbot/src/logic/playerMovement.test.rs
  ```
- **Typical Test Structure:**
  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn test_move_player() {
          // Test implementation
      }
  }
  ```

## Commands

| Command            | Purpose                                               |
|--------------------|-------------------------------------------------------|
| /new-logic-module  | Scaffold and register a new logic module in the codebase |
```
