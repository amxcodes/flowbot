# Skills Management TUI Tab

Add a dedicated "Skills" tab to the TUI interface for easy visual management of skills.

## Implementation Plan

### 1. Update Tab Enum
Add `Skills` variant to the `Tab` enum in `src/tui.rs`.

### 2. Skills State
```rust
pub struct SkillsState {
    pub skills: Vec<SkillInfo>,
    pub selected_index: usize,
}

pub struct SkillInfo {
    pub name: String,
    pub enabled: bool,
    pub has_credentials: bool,
    pub tool_count: usize,
    pub category: String,
}
```

### 3. UI Layout
```
┌─ Skills ────────────────────────────────────────┐
│ [Tab 1: Chat] [Tab 2: Memory] [Tab 3: Skills]  │
├─────────────────────────────────────────────────┤
│ Skill Name      Status    Tools  Category       │
│ > github        ✓ Enabled    3   automation     │
│   weather       ✓ Enabled    2   productivity   │
│   notion        ✗ Disabled   3   productivity   │
│   spotify       ✓ Enabled    1   productivity   │
│   calendar      ✗ Disabled   3   productivity   │
│                                                  │
│ [e] Enable  [d] Disable  [s] Setup              │
└─────────────────────────────────────────────────┘
```

### 4. Keybinds
- `Tab` / `1-3` - Switch tabs
- `↑`/`↓` or `j`/`k` - Navigate skills
- `e` - Enable selected skill
- `d` - Disable selected skill  
- `s` - Run setup wizard for selected skill
- `r` - Refresh skills list

### 5. Real-time Updates
- Load skills from `SkillLoader` on tab open
- Check credential status from `SkillsConfig`
- Update display when skill state changes

## Code Changes

### `src/tui.rs`
1. Add `Skills` to `Tab` enum
2. Add `SkillsState` struct
3. Add `render_skills_tab()` function
4. Handle keybinds in event loop
5. Load skills on init

### Integration
- Use `flowbot_rs::skills::SkillLoader` to scan skills
- Use `flowbot_rs::skills::config::SkillsConfig` for state
- Reload on 'r' keypress for live updates

## Benefits
- Visual skill management without CLI
- Quick enable/disable toggle
- See credential configuration status at a glance
- No context switching from TUI
