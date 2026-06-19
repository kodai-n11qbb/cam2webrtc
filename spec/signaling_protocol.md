# シグナリングプロトコル仕様書 (Signaling Protocol)

本ドキュメントは、Cam2WebRTCにおけるREST APIエンドポイント、WebSocket接続、およびシグナリングメッセージの定義とスキーマについて詳細に解説します。

---

## 1. REST API 仕様

サーバーは、クライアントの接続前に必要な初期処理（ルーム作成、接続検証、設定読み込み）を行うための軽量なREST APIを提供しています。

### 1.1 ルーム作成 (Create Room)
配信者（Sender）が配信を開始する前に、新しい一意なルームをサーバー上に確保するために呼び出します。

- **URL**: `POST /api/rooms`
- **リクエスト本文 (JSON)**:
  ```json
  {}
  ```
- **レスポンス本文 (JSON)**:
  - ステータス: `200 OK`
  - 本文:
    ```json
    {
      "room_id": "8b51680d-85fa-4f51-b0be-3f11e9f19ef6"
    }
    ```
- **処理内容**:
  サーバー側で一意なUUID（V4）を生成し、`RoomManager`に新しい`Room`構造体をメモリ確保して返却します。

---

### 1.2 ルームの存在確認 (Check Room Connection)
視聴者（Viewer）が入力されたルームIDがサーバー上に存在するかどうかを検証するために呼び出します。

- **URL**: `GET /api/rooms/{room_id}`
- **URLパラメータ**:
  - `room_id` (String): 検証対象のルームUUID
- **レスポンス本文 (JSON)**:
  - ルームが存在する場合:
    - ステータス: `200 OK`
    - 本文: `{"exists": true}`
  - ルームが存在しない場合:
    - ステータス: `404 Not Found`
    - 本文: `{"error": "Not Found"}`

---

### 1.3 設定情報の取得 (Get Config)
クライアント（Sender/Viewer）が起動時に、ICEサーバー（STUN/TURN）の接続URLやビデオ解像度の設定パラメータを取得するために呼び出します。

- **URL**: `GET /api/config`
- **リクエストヘッダー (任意)**:
  - `Host`: サーバーのホスト名またはIPアドレス
- **レスポンス本文 (JSON)**:
  - ステータス: `200 OK`
  - 本文例:
    ```json
    {
      "signaling_addr": "0.0.0.0:8080",
      "stun_addr": "0.0.0.0:3478",
      "turn_addr": "0.0.0.0:3479",
      "ice_servers": [
        {
          "urls": [
            "stun:192.168.1.100:3478"
          ]
        }
      ],
      "video_constraints": {
        "width": {
          "ideal": 1280
        },
        "height": {
          "ideal": 720
        }
      },
      "tls_enabled": true,
      "tls_cert_path": "cert.pem",
      "tls_key_path": "key.pem"
    }
    ```
- **自動IP置換処理**:
  リクエスト受信時、サーバーは自身のローカルIPアドレス（例: `192.168.X.X`）を動的に検出します。`ice_servers` 内の `urls` に `localhost` または `127.0.0.1` が含まれている場合、検出した実際のローカルIPアドレスに自動置換してクライアントに返します。これにより、モバイルなどの別端末から接続する際のICE解決を可能にしています。

---

## 2. WebSocket シグナリング仕様

すべてのシグナリング（WebRTC接続交渉メッセージの交換）は、WebSocketを介して双方向かつ低遅延で行われます。

- **エンドポイント**: `ws://{host}/ws/{room_id}` または `wss://{host}/ws/{room_id}`
- **データ形式**: JSONテキスト

---

### 2.1 共通メッセージスキーマ

送受信されるすべてのメッセージは、以下の構造体（`SignalingMessage`）に準拠したJSONフォーマットです。

| フィールド名 | 型 | 必須/任意 | 説明 |
| :--- | :--- | :--- | :--- |
| `type` | String | **必須** | メッセージタイプ (`join`, `room_info`, `new_peer`, `leave`, `offer`, `answer`, `ice_candidate`, `error`) |
| `connection_id` | String | 任意 | メッセージの宛先、または自身の接続ID |
| `sender_id` | String | 任意 | メッセージの送信元の接続ID |
| `offer_id` | String | 任意 | オファーを識別するためのUUID（レガシーモード用） |
| `data` | Value | 任意 | メッセージのタイプに応じた具体的なペイロードオブジェクト |
| `is_sender` | Boolean | 任意 | 接続開始時に配信者であるかを指定するフラグ |

---

### 2.2 各メッセージ詳細とスキーマ

#### 1. Join (参加要求)
クライアントがWebSocket接続を確立した直後に、ルームへ登録するために送信します。

- **送信元**: クライアント（配信者・視聴者両方）
- **JSONフォーマット**:
  ```json
  {
    "type": "join",
    "connection_id": "sender_a1b2c3d4",
    "is_sender": true
  }
  ```
  *(視聴者の場合は `is_sender: false`、`connection_id` はブラウザ側で自動生成された一意な文字列を指定)*

---

#### 2. Room Info (ルームステータス同期)
`join` メッセージへの応答として、サーバーが参加クライアントに対してルーム内の現在の状態を送り返します。

- **送信元**: サーバー
- **JSONフォーマット**:
  ```json
  {
    "type": "room_info",
    "connection_id": "sender_a1b2c3d4",
    "data": {
      "room_id": "8b51680d-85fa-4f51-b0be-3f11e9f19ef6",
      "mode": "1onN",
      "connection_count": 1,
      "peers": []
    }
  }
  ```
  *(すでに他の視聴者が存在している場合は、`peers` 配列に `{ "id": "viewer_xxx", "is_sender": false }` が格納されます)*

---

#### 3. New Peer (新規ピア参加通知)
新しいピアがルームに参加した際、そのルームに既に接続している他のすべてのピアに対して送信されます。

- **送信元**: サーバー
- **JSONフォーマット**:
  ```json
  {
    "type": "new_peer",
    "connection_id": "sender_a1b2c3d4",
    "data": {
      "connection_id": "viewer_z9y8x7w6",
      "is_sender": false,
      "connection_count": 2
    }
  }
  ```
- **クライアントのアクション**:
  配信者がこのメッセージ（`is_sender: false`の新規視聴者）を受信すると、該当の `connection_id` に向けて自動的にWebRTC接続（Offerの生成）を開始します。

---

#### 4. Offer (SDPオファー送信)
WebRTC接続を開始するため、配信者が特定の視聴者に対して接続仕様（SDP）を送信します。

- **送信元**: 配信者（Sender）
- **JSONフォーマット**:
  ```json
  {
    "type": "offer",
    "connection_id": "viewer_z9y8x7w6",
    "sender_id": "sender_a1b2c3d4",
    "data": {
      "type": "offer",
      "sdp": "v=0\r\no=- 42345 2 IN IP4 127.0.0.1\r\n..."
    }
  }
  ```
- **ルーティング**: サーバーは `connection_id` に設定されている視聴者IDを特定し、そのWebSocket接続へこのメッセージをそのまま転送します。

---

#### 5. Answer (SDPアンサー返信)
オファーを受け取った視聴者が、自身のWebRTC接続仕様（SDP）を配信者に返送します。

- **送信元**: 視聴者（Viewer）
- **JSONフォーマット**:
  ```json
  {
    "type": "answer",
    "connection_id": "sender_a1b2c3d4",
    "sender_id": "viewer_z9y8x7w6",
    "data": {
      "type": "answer",
      "sdp": "v=0\r\no=- 87654 2 IN IP4 127.0.0.1\r\n..."
    }
  }
  ```
- **ルーティング**: サーバーは `connection_id` に設定されている配信者IDに向けて転送します。

---

#### 6. ICE Candidate (ICE候補交換)
P2P接続経路（IP、ポート、プロトコル候補）をやり取りするために配信者と視聴者間で複数回送信されます。

- **送信元**: 配信者または視聴者
- **JSONフォーマット**:
  ```json
  {
    "type": "ice_candidate",
    "connection_id": "viewer_z9y8x7w6",
    "sender_id": "sender_a1b2c3d4",
    "data": {
      "candidate": "candidate:842130496 1 udp 16777215 192.168.1.100 51432 typ srflx raddr 192.168.1.100 rport 51432 ...",
      "sdpMid": "0",
      "sdpMLineIndex": 0
    }
  }
  ```
- **ルーティング**: `connection_id` で指定されたターゲットピア宛にサーバーが転送します。

---

#### 7. Leave (退出通知)
ピアがWebSocketを切断（ブラウザを閉じる、または明示的に切断）した際、同じルームの他すべてのピアへ通知されます。

- **送信元**: サーバー
- **JSONフォーマット**:
  ```json
  {
    "type": "leave",
    "connection_id": "sender_a1b2c3d4",
    "data": {
      "connection_id": "viewer_z9y8x7w6",
      "connection_count": 1
    }
  }
  ```
- **クライアントのアクション**:
  配信者はこのメッセージを受け取ると、離脱した `connection_id` に紐づく `RTCPeerConnection` をクローズしてメモリから解放します。

---

#### 8. Error (エラー通知)
シグナリング中に不整合（例: すでに配信者が存在するルームに、別の配信者が参加しようとした場合など）が発生した場合に送信されます。

- **送信元**: サーバー
- **JSONフォーマット**:
  ```json
  {
    "type": "error",
    "connection_id": "sender_another",
    "data": {
      "error": "Sender already exists in this room"
    }
  }
  ```
