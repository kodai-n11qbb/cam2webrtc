# Cam2WebRTC

Rustで実装されたWebRTCシグナリングおよび静的ファイル配信サーバー。カメラ映像をローカルネットワーク（LAN）内で低遅延配信するための完全オフライン対応システム。

---

## 1. 特徴

- **1onN配信 (P2P Mesh)**: 配信者から複数の視聴者に対して、サーバーを介さず直接映像ストリームを送信するMesh型トポロジー。
- **エントランスポータル (`index.html`)**: 配信者・視聴者用のUI画面へスムーズに遷移するためのグラスモルフィズムデザインを採用したダッシュボード。
- **内蔵STUNサーバー**: NAT通過（ICE解決）に必要なIP・ポートのペアを検出するUDPサービス（Port: 3478）。
- **内蔵TURNサーバー (模擬実装)**: クライアント間のメディアデータを中継するためのUDPサービス（Port: 3479）。
  - *※注意: 現行実装ではパケットのデコードとログ出力のみ行われ、実際のパケット転送（リレー）処理は未実装です。*
- **TLS自己署名証明書の自動生成**: 起動時に自動でSSL/TLS証明書を生成。同一ネットワーク内のモバイルデバイス（iOS/Android）からカメラを安全に起動（Secure Context要件のクリア）するためのSAN（Subject Alternative Name）設定に対応。
- **完全オフライン動作**: インターネット接続が不要で、ローカルLAN環境のみで動作。

---

## 2. クイックスタート

### 2.1 起動
リポジトリのルートで以下のコマンドを実行するだけで、自動的にビルドされてサーバーが起動します。
```bash
cargo run
```
初回起動時には、自動的に自己署名証明書（`cert.pem` / `key.pem`）が生成されます。

### 2.2 設定のカスタマイズ (任意)
動作ポートや解像度の設定を変更したい場合は、ルートディレクトリの `config.json` を編集します。
```json
{
  "signaling_addr": "0.0.0.0:8080",
  "stun_addr": "0.0.0.0:3478",
  "turn_addr": "0.0.0.0:3479",
  "ice_servers": [
    {
      "urls": ["stun:localhost:3478"]
    }
  ],
  "video_constraints": {
    "width": { "ideal": 1280 },
    "height": { "ideal": 720 }
  },
  "tls_enabled": true,
  "tls_cert_path": "cert.pem",
  "tls_key_path": "key.pem"
}
```

### 2.3 リリースバイナリのビルド (任意)
常時稼働や配布のために最適化されたバイナリを作成したい場合は、リリースビルドを実行します。
```bash
cargo build --release
```
生成されたバイナリは `target/release/cam2webrtc` に出力されます。

### 2.4 アクセス
ブラウザ（Google Chrome, Safari, Chromium等）で以下のURLを開きます。
- **エントランスポータル**: `https://localhost:8080/`
  ポータル画面から「配信者画面（`sender.html`）」または「視聴者画面（`viewer.html`）」へ簡単にアクセスできます。
- 同一LAN内のスマホなどからアクセスする場合は、起動時にコンソールへ表示されるラズパイやPCのローカルIPアドレス（`https://192.168.x.x:8080/`）へアクセスし、自己署名証明書の警告を許可して進んでください。

---

## 3. プロジェクト構成

```
.
├── src/
│   ├── main.rs                  # エントリーポイント・Warpルート・シグナリング管理
│   ├── signaling.rs             # シグナリングメッセージ定義
│   ├── room.rs                  # ルーム・接続情報管理
│   ├── stun.rs                  # STUNサーバーバイナリプロトコル実装
│   ├── turn.rs                  # TURNサーバー割り当て・模擬パケット処理
│   ├── network.rs               # ローカルIP自動検出・ユーティリティ
│   └── config.rs                # 設定ファイル読み込み
├── static/
│   ├── index.html               # ナビゲーションポータル（エントランス）
│   ├── sender.html              # 配信者（カメラ映像ソース）UI
│   ├── viewer.html              # 視聴者（プレイヤー）UI
│   └── global.css               # アプリ全体の共通スタイルシート（デザインシステム）
├── spec/                        # 技術・構成仕様書
│   ├── system_architecture.md   # システムアーキテクチャ・シーケンス
│   ├── signaling_protocol.md    # REST API & WebSocket メッセージ定義
│   ├── stun_turn_specification.md # STUN/TURN 詳細動作 & 制限事項
│   ├── client_implementation.md # クライアント側JS仕様 & デザインシステム
│   ├── configuration_and_deployment.md # 設定・TLS・スマホ警告回避手順
│   └── raspberrypi_setup.md     # Raspberry Piへの導入・自動起動手順
├── config.json                  # 設定ファイル
├── Cargo.toml                   # Rust依存パッケージ管理
└── LICENSE                      # MITライセンス
```

---

## 4. 詳細仕様書について

システムの詳細な内部設計や動作仕様、環境構築の手順については、[spec/](spec/) ディレクトリ内の各種ドキュメントを参照してください。

1. **[システム構成・アーキテクチャ仕様書](spec/system_architecture.md)**
   - 全体の論理アーキテクチャ、非同期モデル、WebRTC接続時の詳細なメッセージシーケンス図。
2. **[シグナリングプロトコル仕様書](spec/signaling_protocol.md)**
   - REST API（`/api/rooms`, `/api/config`）のJSONフォーマット、WebSocketでの各シグナリングメッセージ（`join`, `offer`, `answer`, `ice_candidate`等）のスキーマ定義。
3. **[STUN/TURN サーバー仕様書](spec/stun_turn_specification.md)**
   - UDPパケット構造とデコード、XORによるポート/アドレス暗号化ロジック、および内蔵TURN中継機能の模擬制限に関する記述。
4. **[クライアント実装仕様書](spec/client_implementation.md)**
   - クライアント側WebRTC制御の流れ、Mesh型マルチピア接続、自動再接続ポーリング、および `global.css` をベースとしたデザイン規格。
5. **[設定とデプロイ仕様書](spec/configuration_and_deployment.md)**
   - 動作に必要な開放ポート一覧、TLS/HTTPS接続のセキュリティ制約（Secure Context要件）、iOS/Android端末からの証明書警告スキップ手順。
6. **[Raspberry Pi 導入・セットアップ仕様書](spec/raspberrypi_setup.md)**
   - ラズパイ上でのネイティブコンパイルおよび開発PCからのクロスコンパイル手順、システム起動時に自動常時バックグラウンド実行するための `Systemd` の構築手順。
