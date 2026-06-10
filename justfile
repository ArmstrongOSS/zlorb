default: install

install-bins:
  cargo install --path zlorb-ctl
  cargo install --path zlorb-service
  ln -sf ~/.cargo/bin/zlorb-service /usr/bin/zlorb-service

install-service:
  cp zlorb.service /usr/lib/systemd/system/zlorb.service
  systemctl daemon-reload
  systemctl enable zlorb.service --now

install: install-bins install-service

clean:
  cargo clean

ctl:
  cargo run -p zlorb-ctl

service:
  cargo run -p zlorb-service

web:
  cargo run -p zlorb-web