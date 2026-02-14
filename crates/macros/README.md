# Exiv Plugin Macros

Procedural macros for defining Exiv plugins with automatic manifest generation, icon embedding, and capability registration.

## Usage

```rust
use exiv_macros::exiv_plugin;
use exiv_shared::{Plugin, PluginConfig, PluginFactory};

#[exiv_plugin(
    name = "my.plugin",
    kind = "Tool",
    description = "Example plugin",
    version = "0.1.0",
    icon = "assets/icon.svg",
    permissions = ["FileRead", "FileWrite"],
    capabilities = ["Tool"]
)]
pub struct MyPlugin {
    // plugin fields
}

impl MyPlugin {
    pub async fn new_plugin(config: PluginConfig) -> anyhow::Result<Self> {
        Ok(Self {})
    }
}

#[async_trait::async_trait]
impl Plugin for MyPlugin {
    // implement plugin trait
}
```

## Performance Optimization

### Development Mode: Skip Icon Embedding

During development, you can skip icon embedding to reduce compilation time:

```bash
export EXIV_SKIP_ICON_EMBED=1
cargo build
```

This will:
- Skip reading and Base64 encoding icon files
- Reduce macro expansion overhead
- Speed up incremental builds
- Set `icon_data` to `None` in manifest

**Use for**: Local development, CI test builds, rapid iteration

**Do NOT use for**: Production builds, release packages

### Production Mode: Full Icon Embedding

For production builds, ensure icons are embedded:

```bash
unset EXIV_SKIP_ICON_EMBED
cargo build --release
```

Or explicitly disable the flag:

```bash
EXIV_SKIP_ICON_EMBED=0 cargo build --release
```

## Macro Attributes

### Required Fields

- `name`: Plugin identifier (e.g., `"mind.deepseek"`)
- `kind`: Service type (`Reasoning`, `Memory`, `Tool`, `HAL`, `Communication`)
- `description`: Human-readable description

### Optional Fields

- `version`: SemVer string (default: `"0.1.0"`)
- `category`: Plugin category (default: inferred from `kind`)
- `icon`: Path to icon file (relative to crate root, max 64KB)
- `action_icon`: Lucide icon name for UI actions
- `permissions`: Array of required permissions
- `capabilities`: Array of provided capabilities
- `config_keys`: Array of required configuration keys
- `tags`: Array of custom tags

## Icon Requirements

- **Format**: SVG, PNG, or JPG
- **Size**: Maximum 64KB
- **Path**: Relative to plugin crate's `Cargo.toml`
- **Encoding**: Automatically converted to Base64 at compile time

Example icon paths:
```rust
icon = "assets/icon.svg"           // ✅ Relative path
icon = "../shared/common-icon.png" // ✅ Parent directory
icon = "/tmp/icon.svg"             // ❌ Absolute path (not portable)
```

## Compile-Time Validation

The macro performs validation at compile time:

1. **Required fields**: `name`, `kind`, `description` must be non-empty
2. **Icon size**: Icons larger than 64KB will cause compilation error
3. **Icon accessibility**: Icon file must be readable at compile time
4. **Service type**: Must match a valid `ServiceType` variant

## Generated Code

The macro generates:

1. **Plugin constant**: `MyPlugin::PLUGIN_ID`
2. **Factory method**: `MyPlugin::factory()`
3. **Auto manifest**: `MyPlugin::auto_manifest()`
4. **Downcast methods**: `as_reasoning()`, `as_memory()`, etc.
5. **Factory struct**: `MyPluginFactory`
6. **Inventory submission**: Automatic registration via `inventory` crate

## Build Performance Tips

### For Large Projects

If you have many plugins and want faster incremental builds:

```bash
# Development: skip icon embedding
export EXIV_SKIP_ICON_EMBED=1

# Parallel builds
cargo build -j$(nproc)

# Release: full build with icons
unset EXIV_SKIP_ICON_EMBED
cargo build --release
```

### CI/CD Pipelines

```yaml
# GitHub Actions example
- name: Fast test build
  env:
    EXIV_SKIP_ICON_EMBED: 1
  run: cargo test

- name: Production build
  run: cargo build --release
```

## Error Messages

### Icon Too Large

```
error: 🔌 Plugin Icon 'assets/large-icon.png' is too large (128000 bytes). Limit is 64KB.
```

**Solution**: Compress the icon or use a smaller file.

### Missing Required Field

```
error: Plugin 'name' is required
```

**Solution**: Add the `name = "..."` attribute.

### Icon Not Found

```
error: 🔌 Failed to read icon at 'assets/missing.svg': No such file or directory
```

**Solution**: Verify icon path is relative to crate root and file exists.

## License

Same as Exiv project license.
