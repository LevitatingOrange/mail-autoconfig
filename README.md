# mail-autoconfig

## Invocation
```sh
    cargo run -- --config-file=default_config.toml run
```

## Docker Image
* Build with `docker build ./`
* Default config file path is `/srv/config.toml`
* You can reload the state of the program by running `reload-state` inside the container, e.g.
```sh
docker exec -it <container-id> reload-state
```

