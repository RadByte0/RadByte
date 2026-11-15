# Radonfigs

Radon configuration files (`*.toml`) are stored in `radonfigs/` within the service's working directory, for example `dist/RadServer/radonfigs/`.

- If a `<radon_name>.toml` file exists in `radonfigs/`, `RadProxy` automatically loads it at startup and during `reload` (`RadonManager::config_for("<radon_name>")`).
- Declarative `RadonManifest` configs (with fields `id`, `target = "proxy"`, `route_host`, `route_to`) are also picked up automatically.
