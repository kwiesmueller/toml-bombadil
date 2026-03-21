# SyncPlan Structure: Two Options

## Example Scenario

```toml
[settings]
prehooks = ["backup-dotfiles"]
posthooks = ["notify-done"]

[settings.dots.zsh]
source = "zsh/zshrc"
target = "~/.zshrc"
posthooks = ["source ~/.zshrc"]

[settings.dots.zsh-plugins]
source = "zsh/plugins/"
target = "~/.config/zsh/plugins"
depends_on = ["zsh"]

[settings.dots.nvim]
source = "nvim/"
target = "~/.config/nvim"
prehooks = ["mkdir -p ~/.local/share/nvim"]
posthooks = ["nvim --headless +PackerSync +qa"]

[settings.packages.neovim]
install.dnf = "neovim"
posthooks = ["nvim --headless +checkhealth +qa"]

[settings.packages.zsh]
install.dnf = "zsh"
```

---

## Option A: Nested (hooks embedded in their owner)

### Data structure

```rust
pub struct SyncPlan {
    pub global_prehooks: Vec<PlannedHook>,
    pub items: Vec<PlannedItem>,          // topologically sorted
    pub global_posthooks: Vec<PlannedHook>,
}

pub struct PlannedItem {
    pub prehooks: Vec<PlannedHook>,
    pub action: ItemAction,
    pub posthooks: Vec<PlannedHook>,
}

pub enum ItemAction {
    Dot(PlannedDot),
    Package(PlannedPackage),
}
```

Dependency ordering is expressed by **position in `items`** — zsh-plugins comes after zsh because
the topological sort placed it there. The nesting here is *hooks-within-item*, not items-within-items.

### Dry-run display

```
bombadil sync --dry-run

  Global pre
  ├─ hook  backup-dotfiles

  Dot: zsh
  ├─ create  ~/.zshrc  ←  zsh/zshrc
  └─ hook    source ~/.zshrc

  Dot: zsh-plugins  (after: zsh)
  └─ create  ~/.config/zsh/plugins/  ←  zsh/plugins/

  Dot: nvim
  ├─ hook    mkdir -p ~/.local/share/nvim
  ├─ create  ~/.config/nvim/  ←  nvim/
  └─ hook    nvim --headless +PackerSync +qa

  Package: zsh
  └─ install  zsh  (dnf)

  Package: neovim
  ├─ install  neovim  (dnf)
  └─ hook     nvim --headless +checkhealth +qa

  Global post
  └─ hook  notify-done

  3 dots (3 create), 2 packages (2 install), 4 hooks
```

### Executor

```rust
fn execute_plan(plan: SyncPlan, storage: &AuditStorage) -> Result<Session> {
    for hook in &plan.global_prehooks {
        run_hook(hook, &mut session, storage)?;
    }
    for item in &plan.items {
        for hook in &item.prehooks {
            run_hook(hook, &mut session, storage)?;
        }
        match &item.action {
            ItemAction::Dot(d)     => install_dot(d, &mut session, storage)?,
            ItemAction::Package(p) => install_package(p, &mut session, storage)?,
        }
        for hook in &item.posthooks {
            run_hook(hook, &mut session, storage)?;
        }
    }
    for hook in &plan.global_posthooks {
        run_hook(hook, &mut session, storage)?;
    }
    Ok(session)
}
```

---

## Option B: Flat ordered list

### Data structure

```rust
pub struct SyncPlan {
    pub steps: Vec<SyncStep>,   // fully ordered, all hooks inlined
}

pub enum SyncStep {
    Hook(PlannedHook),
    Dot(PlannedDot),
    Package(PlannedPackage),
}

// Hooks carry their owner for display purposes only
pub struct PlannedHook {
    pub command: String,
    pub owner: HookOwner,
    pub phase: HookPhase,
}
pub enum HookOwner { Global, Dot(String), Package(String) }
pub enum HookPhase { Pre, Post }
```

The relationship between a hook and its dot is recorded in the hook's metadata, not in the
structure itself.

### Dry-run display (same output, reconstructed from metadata)

```
bombadil sync --dry-run

  hook     backup-dotfiles              [global pre]
  create   ~/.zshrc  ←  zsh/zshrc      [dot: zsh]
  hook     source ~/.zshrc              [dot: zsh, post]
  create   ~/.config/zsh/plugins/  ←   [dot: zsh-plugins, after: zsh]
  hook     mkdir -p ~/.local/share/nvim [dot: nvim, pre]
  create   ~/.config/nvim/  ←  nvim/   [dot: nvim]
  hook     nvim --headless +PackerSync  [dot: nvim, post]
  install  zsh  (dnf)                  [package: zsh]
  install  neovim  (dnf)               [package: neovim]
  hook     nvim --headless +checkhealth [package: neovim, post]
  hook     notify-done                  [global post]

  3 dots (3 create), 2 packages (2 install), 4 hooks
```

### Executor

```rust
fn execute_plan(plan: SyncPlan, storage: &AuditStorage) -> Result<Session> {
    for step in &plan.steps {
        match step {
            SyncStep::Hook(h)    => run_hook(h, &mut session, storage)?,
            SyncStep::Dot(d)     => install_dot(d, &mut session, storage)?,
            SyncStep::Package(p) => install_package(p, &mut session, storage)?,
        }
    }
    Ok(session)
}
```

---

## Comparison

| | Option A: Nested | Option B: Flat |
|---|---|---|
| **Executor** | Three loops per item; slightly more code | One loop; minimal code |
| **Dry-run display** | Groups naturally; reads top-to-bottom like a script | Linear; easier to reason about exact execution order |
| **Plan builder** | Must keep hooks attached to their item through sort | Hooks are just steps; sort only reorders non-hook steps |
| **Adding a step type** | Add a new `ItemAction` variant | Add a new `SyncStep` variant |
| **Reordering risk** | Hooks can never be separated from their item | Hooks could drift from their item if sort logic is wrong |
| **Reflects actual order** | Implicit (follow the nesting) | Explicit (step N runs before step N+1) |
| **Serializing plan to disk** | Straightforward | Straightforward |

---

## Recommendation

**Option A** is the right model for the *data representation* (plan builder, audit storage, tests).
The ownership relationship between a hook and its dot/package is structural — encoding it in the
type system means the compiler enforces it. A hook can never be accidentally orphaned.

**Option B's display style** (flat linear sequence with labels) can still be produced *from* Option
A by flattening during rendering. This gives the best of both: structural correctness in the model,
linear readability in the output.

The executor for Option A is three small loops and reads exactly like the design doc. The executor
for Option B is one loop but only works correctly because the plan builder got the order right —
there's no structural guarantee.

### Recommended final design

```rust
// Plan structure: nested (Option A)
pub struct SyncPlan {
    pub global_prehooks: Vec<PlannedHook>,
    pub items: Vec<PlannedItem>,
    pub global_posthooks: Vec<PlannedHook>,
}

// Display: flatten to ordered steps for dry-run output
impl SyncPlan {
    pub fn steps(&self) -> impl Iterator<Item = DisplayStep<'_>> {
        // yields global pre → (item pre, item action, item post)* → global post
    }
}
```
