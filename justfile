default: install

install-bins:
  cargo install --path zlorbrs-ctl
  cargo install --path zlorbrs-service
  ln -sf ~/.cargo/bin/zlorbrs-service /usr/bin/zlorbrs-service

install-service:
  cp zlorbrs.service /usr/lib/systemd/system/zlorbrs.service
  systemctl daemon-reload
  systemctl enable zlorbrs.service --now

install: install-bins install-service

clean:
  cargo clean
