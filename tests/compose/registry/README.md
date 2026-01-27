# Registry integration test stack

This docker-compose stack provides two local registries:
- Anonymous registry: `127.0.0.1:5000`
- Basic auth registry: `127.0.0.1:5001` (user: `testuser`, pass: `testpassword`)

## Start
```bash
docker compose up -d
```

## Run integration tests
```bash
REGISTRY_INTEGRATION=1 cargo test -p containerregistry-registry --test integration_registry
```

Optional overrides:
- `REGISTRY_ANON_ADDR` (default `127.0.0.1:5000`)
- `REGISTRY_BASIC_ADDR` (default `127.0.0.1:5001`)

## Stop
```bash
docker compose down -v
```
