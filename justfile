default: install

install-bins:
  cargo install --path zlorb-ctl
  cargo install --path zlorb-service

install-service:
  zlorb-ctl setup

install: install-bins install-service

clean:
  cargo clean

ctl:
  cargo run -p zlorb-ctl

service:
  cargo run -p zlorb-service

web:
  cargo run -p zlorb-ctl web  
