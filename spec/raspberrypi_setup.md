# Raspberry Pi 導入・セットアップ仕様書 (Raspberry Pi Setup Guide)

本ドキュメントは、Cam2WebRTCアプリケーションをRaspberry Pi上でコンパイル、実行、運用するための手順および技術仕様を定義します。本システムは、サーバー機能（シグナリングおよびSTUN/TURN）のみを動作させる「サーバー専用ロール」と、カメラモジュールを接続して配信も行う「配信機兼サーバーロール」の両方に対応しています。

---

## 1. ハードウェア・OS要件

### 推奨ハードウェア
- **Raspberry Pi 3 Model B+ / 4 Model B / 5**、または **Raspberry Pi Zero 2 W**
- **配信機ロール時**: Raspberry Pi Camera Module (V2/V3) または、汎用のUVC対応USBカメラ

### 推奨OS
- **Raspberry Pi OS (64-bit)** (Debian Bookwormベース)
  - 理由: Rustの `aarch64` ターゲットがネイティブで完全にサポートされており、パフォーマンスが最大化されます。

---

## 2. ビルド・コンパイル手順

コンパイル方法には、ラズパイ上で直接行う「ネイティブコンパイル」と、高速なPC/Macでビルドして転送する「クロスコンパイル」の2通りがあります。

### 2.1 ラズパイ上でのネイティブコンパイル
ラズパイのスペックに余裕がある場合（Pi 4 (4GB以上) や Pi 5推奨）、最も簡単な手順です。

1. **Rustツールチェーンのインストール**:
   ラズパイのターミナルで以下を実行します。
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   source $HOME/.cargo/env
   ```
2. **ビルドの実行**:
   リポジトリクローン後、リリースビルドを実行します。
   ```bash
   cargo build --release
   ```
   ※ メモリが1GB以下のデバイス（Zero 2 Wなど）では、コンパイル時にメモリ不足になる場合があります。その場合はスワップ領域を増やすか、以下のクロスコンパイルを実行してください。

---

### 2.2 ホストPCからのクロスコンパイル (推奨)
開発用のMacやWindowsから、ラズパイ用のバイナリを高速にビルドします。クロスコンパイルツール `cross` を使用すると、コンテナ環境を利用してリンカエラーを防ぐことができます。

1. **Dockerの起動**:
   ホストPCでDocker Desktop等を起動しておきます。
2. **`cross` のインストール**:
   ```bash
   cargo install cross --git https://github.com/cross-rs/cross
   ```
3. **ターゲットに合わせたビルド実行**:
   - **Raspberry Pi OS 64-bit (aarch64) 用**:
     ```bash
     cross build --target aarch64-unknown-linux-gnu --release
     ```
   - **Raspberry Pi OS 32-bit (armv7) 用**:
     ```bash
     cross build --target armv7-unknown-linux-gnueabihf --release
     ```
4. **バイナリおよびアセットのラズパイへの転送**:
   ビルドされたバイナリと必要なアセットを `scp` で転送します。
   ```bash
   # 64-bitバイナリの転送例
   scp target/aarch64-unknown-linux-gnu/release/cam2webrtc pi@<ラズパイのIPアドレス>:~/
   scp config.json pi@<ラズパイのIPアドレス>:~/
   scp -r static pi@<ラズパイのIPアドレス>:~/
   ```

---

## 3. カメラのセットアップ (配信機ロール用)

ラズパイ自体をカメラ配信の送信元（Sender）にする場合の追加設定です。

### 3.1 ハードウェア認識の確認
1. カメラモジュールまたはUSBカメラを接続します。
2. カメラが認識されているか確認します。
   - **Pi Cameraモジュール (libcameraスタック)**:
     ```bash
     libcamera-hello --list-cameras
     ```
     ※「Available cameras:」にカメラが表示されれば正常です。
   - **USBカメラ (V4L2スタック)**:
     ```bash
     ls /dev/video*
     ```
     ※ `/dev/video0` 等のデバイスファイルが表示されれば認識されています。

### 3.2 ブラウザでのカメラ起動（GUI起動時）
ラズパイのデスクトップOS上で Chromium ブラウザを起動して配信を行います。

- **アドレス**: `https://localhost:8080/sender.html`
- **注意点**: 
  `localhost` または `127.0.0.1` からのアクセスであれば、自己署名証明書の警告状態であってもブラウザのセキュリティ制限（Secure Context）を回避してカメラを起動できます。初回起動時にカメラ・マイクの許可ポップアップが出るので「許可」を選択してください。

---

## 4. ネットワークとファイアウォール設定

### 4.1 静的IPアドレスの推奨
同一LAN内の他デバイスから安定してアクセスするために、ラズパイのIPアドレスをルーター側で固定するか、ラズパイの `dhcpcd.conf` または `NetworkManager` で静的IPアドレスを設定することを推奨します。

### 4.2 ファイアウォール (ufw) 設定
ラズパイでファイアウォール（UFW）が有効になっている場合は、以下のポートを開放してください。

```bash
sudo ufw allow 8080/tcp      # HTTP/HTTPS/WebSocket
sudo ufw allow 3478/udp      # STUN
sudo ufw allow 3479/udp      # TURN
sudo ufw allow 49152:65535/udp # TURNリレー範囲 (任意)
sudo ufw reload
```

---

## 5. 自動起動設定 (Systemdによる常時稼働)

サーバーをラズパイの起動時に自動的にバックグラウンド実行させるための設定です。

### 5.1 サービスファイルの作成
`/etc/systemd/system/cam2webrtc.service` を新規作成します。

```ini
[Unit]
Description=Cam2WebRTC Signaling and Media Server
After=network.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi
ExecStart=/home/pi/cam2webrtc
Restart=always
RestartSec=5
StandardOutput=syslog
StandardError=syslog
SyslogIdentifier=cam2webrtc

[Install]
WantedBy=multi-user.target
```
*(※ユーザー名 `pi` や配置パス `/home/pi` は環境に合わせて調整してください。カレントディレクトリに `config.json` と `static/` ディレクトリが存在する必要があります)*

### 5.2 サービスの有効化と開始
```bash
sudo systemctl daemon-reload
sudo systemctl enable cam2webrtc.service
sudo systemctl start cam2webrtc.service
```

### 5.3 ログの確認
```bash
journalctl -u cam2webrtc.service -f
```
