default: install

install-bins:
  cargo install --path zlorb-ctl
  cargo install --path zlorb-service
  sudo ln -sf ~/.cargo/bin/zlorb-service /usr/bin/zlorb-service

install-service:
  @if [ "$(id -u)" = "0" ]; then \
    sudo cp zlorb.service /usr/lib/systemd/system/zlorb.service; \
    sudo systemctl daemon-reload; \
    sudo systemctl enable zlorb.service --now; \
  else \
    mkdir -p ~/.config/systemd/user/; \
    cp zlorb.service ~/.config/systemd/user/zlorb.service; \
    systemctl --user daemon-reload; \
    systemctl --user enable zlorb.service --now; \
  fi

install: install-bins install-service

clean:
  cargo clean

ctl:
  cargo run -p zlorb-ctl

service:
  cargo run -p zlorb-service

web:
  cargo run -p zlorb-ctl web  
