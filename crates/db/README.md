# everruns-db

> Database utilities for Everruns: embedded SQLite connections and the
> `UpdateField` update type.

[![Crates.io](https://img.shields.io/crates/v/everruns-db.svg)](https://crates.io/crates/everruns-db)
[![Documentation](https://docs.rs/everruns-db/badge.svg)](https://docs.rs/everruns-db)
[![License](https://img.shields.io/crates/l/everruns-db.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

Part of the [Everruns](https://everruns.com) agent framework. Most applications
reach it through the `everruns` crate's `local` feature rather than directly.

```sh
cargo add everruns-db --features sqlite
```

## What It Provides

- `UpdateField<T>`: `Unchanged`, `Clear`, or `Set(T)` for update operations,
  in place of `Option<Option<T>>`.
- `sqlite` (feature `sqlite`): `open`, `open_in_memory`, `backup`, and
  `use_dotfile_locks` over a bundled SQLite. Query callbacks keep the
  `rusqlite::Connection` type through `sqlite::QueryConnection`.

The crate depends on no other Everruns crate.

## Quick Example

```rust
use everruns_db::UpdateField;

let mut title = Some("draft".to_string());
UpdateField::Set("final".to_string()).apply(&mut title);
assert_eq!(title.as_deref(), Some("final"));

UpdateField::<String>::Clear.apply(&mut title);
assert_eq!(title, None);
```

With the `sqlite` feature:

```rust,ignore
let db = everruns_db::sqlite::open_in_memory()?;
db.execute_batch("CREATE TABLE notes (body TEXT)")?;
```

## Documentation

- [Sessions and local persistence](https://docs.everruns.com/framework/sessions/)
- [Upgrade notes](https://docs.everruns.com/framework/upgrade-notes/)
- [API reference](https://docs.rs/everruns-db)

## License

MIT
