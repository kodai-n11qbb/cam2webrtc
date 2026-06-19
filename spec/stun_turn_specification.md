# STUN/TURN サーバー仕様書 (STUN/TURN Servers)

本ドキュメントは、Cam2WebRTCプロジェクトに内蔵されているカスタムSTUNサーバーおよびTURNサーバーのプロトコル実装、動作仕様、およびその制限事項について詳細に定義します。

---

## 1. 内蔵 STUN サーバー仕様

STUN（Session Traversal Utilities for NAT: RFC 5389）は、クライアントが自身のパブリック（またはLAN上の外部）IPアドレスとポート番号を検出するために使用するプロトコルです。

### 1.1 基本情報
- **通信プロトコル**: UDP
- **デフォルトポート**: `3478`
- **RFC対応度**: RFC 5389の最小サブセット（Binding Request/Responseのみ）

### 1.2 パケット構造とハンドリング
STUNサーバーは、バインドされたUDPソケットからメッセージを受信し、ヘッダーの最初の20バイトを解析します。

1. **メッセージ長チェック**: 受信パケットが20バイト未満の場合は無視します。
2. **メッセージタイプ判定**: 最初の2バイトを解析します。
   - `0x0001` (Binding Request): 処理を継続し、バインディング応答を返します。
   - それ以外: `0x0111` (Binding Error Response) を返します（エラーコード 400: Bad Request）。
3. **Magic Cookie & トランザクションID**:
   - リクエストヘッダーの4バイト目から20バイト目のデータ（Magic Cookie `0x2112A442` および一意のトランザクションID）をそのままレスポンスにコピーして返します。

### 1.3 XOR-MAPPED-ADDRESS 属性 (`0x0020`) のエンコード
NATデバイスによるアドレス書き換え対策として、IPアドレスとポートをXOR暗号化した `XOR-MAPPED-ADDRESS` 属性を返却します。

- **ポートの暗号化**:
  クライアントの送信元ポート番号に対して、Magic Cookieの上位16ビット（`0x2112`）でXOR演算を行います。
  $$\text{Encoded Port} = \text{Source Port} \oplus \text{0x2112}$$
- **IPv4アドレスの暗号化**:
  クライアントの送信元IPv4アドレス（4バイト）の各オクテットに対して、Magic Cookie（`0x2112A442`）の対応する各バイトでXOR演算を行います。
  - 第1オクテット $\oplus$ `0x21`
  - 第2オクテット $\oplus$ `0x12`
  - 第3オクテット $\oplus$ `0xA4`
  - 第4オクテット $\oplus$ `0x42`

---

## 2. 内蔵 TURN サーバー仕様

TURN（Traversal Using Relays around NAT: RFC 5766）は、P2Pでの直接通信が困難な場合に、サーバーがメディアパケットの中継を代行するためのプロトコルです。

### 2.1 基本情報
- **通信プロトコル**: UDP
- **デフォルトポート**: `3479`
- **RFC対応度**: 極めて限定的なモック（模擬）実装

### 2.2 リソース割り当てロジック (Allocate Request)
クライアントから `0x0003` (Allocate Request) を受信すると、中継用の一意なポート（リレーアドレス）を割り当てます。

1. **リレーポートの選択**:
   - `49152`（動的ポート範囲の開始値）から順番にインクリメントして割り当てます。
   - `65535` に達した場合は `49152` にラップアラウンド（初期化）します。
2. **アロケーションの保持**:
   - 生成された `Allocation ID`（UUID）とクライアントアドレス、割り当てたリレーアドレス、および有効期限（Lifetime = 600秒）を `allocations` マップに登録します。
3. **レスポンスの生成**:
   - `0x0103` (Allocate Response) を返します。
   - レスポンスには、割り当てたリレーIP/ポートをXORエンコードした `XOR-RELAYED-ADDRESS` 属性（`0x0016`）と、アロケーションの寿命を示す `LIFETIME` 属性（`0x000d`、値は600秒）が含まれます。

---

## 3. 重要：TURNサーバーの実装上の制限事項

> [!WARNING]
> **本プロジェクトの内蔵TURNサーバーは完全なデータ中継に対応していません。**
> 以下の重大な仕様制限があるため、実運用時や厳しいNAT環境下での利用時には注意が必要です。

### 3.1 メディアデータ中継機能の欠落（ログ出力のみ）
クライアントから相手ピアへデータを中継するための `0x0016` (Send Indication) メッセージを受信した際、サーバーはパケット内に含まれる `XOR-PEER-ADDRESS`（中継先アドレス）と `DATA`（送信データ）を正常にデコードします。

しかし、[src/turn.rs](file:///Users/abekoudai/Desktop/cam2webrtc/src/turn.rs) の実装では、**宛先へのUDPパケットの送信（転送処理）が行われません**。
```rust
// turn.rs:L227-234 より抜粋
if let (Some(peer), Some(data_bytes)) = (peer_addr, data) {
    debug!("Relaying data from {} to {}", src_addr, peer);
    
    // In a real implementation, you would forward this data to the peer
    // For now, we just log it
    info!("TURN relay: {} -> {} ({} bytes)", src_addr, peer, data_bytes.len());
}
```
したがって、本TURNサーバーを利用したWebRTCのメディアリレーは機能せず、双方向通信が完全にSTUN経由の直接P2P通信（ホスト・反射アドレス）に依存することになります。

### 3.2 認証機構（Credential）の未実装
標準的なSTUN/TURNサーバーでは、不正利用を防ぐために `MESSAGE-INTEGRITY` 属性を用いたユーザー名とパスワードによる認証（Long-Term Credential Mechanism）が必要ですが、本実装では認証は一切行われません。
そのため、`config.json` の `ice_servers` には認証情報（`username`, `credential`）を含める必要がありません。

### 3.3 チャンネルバインドの未対応
通信効率化のための標準仕様である `ChannelBind`（チャンネル割り当て要求 `0x0009`）には対応していません。
クライアントがチャンネルバインドを試みた場合、サポート外のメッセージタイプとしてエラーが返却されます。
