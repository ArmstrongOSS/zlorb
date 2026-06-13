default: install

install-bins:
  cargo install --path zlorb-ctl
  cargo install --path zlorb-service
  sudo ln -sf ~/.cargo/bin/zlorb-service /usr/bin/zlorb-service

install-service:
  sudo cp zlorb.service /usr/lib/systemd/system/zlorb.service
  sudo systemctl daemon-reload
  sudo systemctl enable zlorb.service --now

install: install-bins install-service

clean:
  cargo clean

ctl:
  cargo run -p zlorb-ctl

service:
  cargo run -p zlorb-service

web:
  bun run --cwd zlorb-web/frontend build && cargo run -p zlorb-web
  
# web: build-frontend
#   cargo run -p zlorb-web

# build-zlorb-web-frontend:
#   bun run --cwd zlorb-web/frontend build
