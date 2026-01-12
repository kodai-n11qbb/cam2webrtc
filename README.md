# Cam2WebRTC

Rustで実装されたWebRTCシグナリングサーバー。カメラ映像をリアルタイム配信するための完全なオフライン対応システム。

## 機能

- **1onN配信 (P2P Mesh)**: 1対多のリアルタイム映像配信（Mesh方式）
- **内蔵STUNサーバー**: ローカルIPアドレス自動検出
- **内蔵TURNサーバー**: 同一LAN内での中継対応
- **WebSocketシグナリング**: 低遅延な通信
- **完全オフライン運用**: インターネット接続不要

## システムアーキテクチャ

```
Sender (HTML) ←→ Rust Signaling Server ←→ Viewer (HTML)
                    ↓
            STUN/TURN Servers
```

## セットアップ

### 1. ビルド

```bash
cargo build --release
```

### 2. 実行

```bash
cargo run
```

サーバーが `http://localhost:8080` で起動します。

### 3. アクセス

- **Sender**: `http://localhost:8080/sender.html`
- **Viewer**: `http://localhost:8080/viewer.html`

## 使用方法

### Sender側

1. `sender.html` にアクセス
2. 「カメラ開始」をクリック
3. 「ルーム作成」をクリック
4. 「配信開始」をクリック
5. 表示されたルームIDをViewerに共有

### Viewer側

1. `viewer.html` にアクセス
2. ルームIDを入力
3. 「ルームに接続」をクリック
4. 自動的に配信が開始される

**自動接続モード**: 常に利用可能なルームを自動検出して接続

## 技術仕様

### サーバー技術

- **Rust**: 高性能なメモリ安全な実装
- **Tokio**: 非同期ランタイム
- **Warp**: HTTP/WebSocketフレームワーク
- **STUN/TURN**: RFC 5389/5766準拠

### クライアント技術

- **WebRTC**: P2P通信
- **WebSocket**: シグナリング
- **MediaStream API**: カメラ/マイクアクセス

### ネットワーク

- **WebSocketポート**: 8080
- **STUNポート**: 3478
- **TURNポート**: 3479

## API

### REST API

#### ルーム作成

```
POST /api/rooms
Content-Type: application/json

{}

Response:
{
  "room_id": "uuid"
}
```

### WebSocket API

#### 接続

```
ws://localhost:8080/ws/{room_id}
```

#### メッセージ形式

```json
{
  "type": "join" | "offer" | "answer" | "ice_candidate" | "room_info" | "error",
  "connection_id": "string",
  "sender_id": "string",
  "offer_id": "string",
  "data": {},
  "is_sender": boolean
}
```

## 開発

### プロジェクト構造

```
src/
├── main.rs          # エントリーポイント
├── signaling.rs     # シグナリングメッセージ
├── room.rs         # ルーム管理
├── stun.rs         # STUNサーバー
└── turn.rs         # TURNサーバー

static/
├── sender.html     # 配信者用HTML
└── viewer.html     # 視聴者用HTML
```

### 依存関係

- `tokio`: 非同期ランタイム
- `warp`: Webフレームワーク
- `serde`: JSONシリアライズ
- `uuid`: 一意ID生成
- `byteorder`: バイトオーダー操作
- `chrono`: 日時処理

## ライセンス

MIT License

## 貢献

プルリクエストを歓迎します。バグ報告や機能要望はIssueで受け付けています。
