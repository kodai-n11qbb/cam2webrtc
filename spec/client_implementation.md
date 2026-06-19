# クライアント実装仕様書 (Client Implementation)

本ドキュメントは、Cam2WebRTCのフロントエンド（ブラウザ側）におけるWebRTCおよびシグナリングの実装仕様について解説します。

---

## 1. クライアント構成概要

フロントエンドは、フレームワークや外部ライブラリを使用しないバニラ（Vanilla）HTML5/JavaScriptで実装されています。

- **ポータルページ (`static/index.html`)**: 配信者ページおよび視聴者ページへスムーズにアクセスするためのエントランス画面です。
- **配信者用ページ (`static/sender.html`)**: カメラ映像を取得し、シグナリングサーバーを介して各視聴者に対して個別にWebRTC Offerを送信します。
- **視聴者用ページ (`static/viewer.html`)**: ルームに参加し、配信者から送られてくるWebRTC Offerを処理してAnswerを返却、映像を再生します。

---

## 2. 共通の初期化処理

### 2.1 設定情報の読み込み
両クライアントともに、読み込み時にサーバーの `GET /api/config` エンドポイントから設定（ICEサーバー情報およびビデオ解像度の制約条件）を取得します。

```javascript
async loadConfig() {
    try {
        const response = await fetch('/api/config');
        if (response.ok) {
            this.config = await response.json();
        }
    } catch (e) {
        console.error('Failed to load config:', e);
    }
}
```

取得した `ice_servers` は、後続の `RTCPeerConnection` 初期化時に以下のように引き渡されます。
```javascript
const pcConfig = {
    iceServers: this.config?.ice_servers || [{ urls: 'stun:localhost:3478' }]
};
const pc = new RTCPeerConnection(pcConfig);
```

---

## 3. 配信者（Sender）の実装仕様

配信者は、ルーム内のすべての視聴者（Viewer）と一対一のP2P接続を確立するため、**マルチピア接続（Mesh接続）**を管理します。

### 3.1 接続状態管理
配信者は、視聴者ごとの `RTCPeerConnection` を管理するためにJavaScriptの `Map` を使用します。
- キー: 視聴者の接続ID (`connection_id`)
- 値: `RTCPeerConnection` インスタンス

```javascript
this.peerConnections = new Map(); // Map<peerId, RTCPeerConnection>
```

### 3.2 カメラ映像のキャプチャ
`navigator.mediaDevices.getUserMedia()` を用いて、カメラとマイクのストリーム（`localStream`）を取得します。解像度は設定ファイルから取得した `video_constraints` が優先して適用されます。

### 3.3 新規視聴者接続時のアクション（Offerの生成）
サーバーから `new_peer` メッセージを受信し、そのピアが視聴者である場合（`is_sender === false`）、配信者は以下の手順を実行します。

```javascript
async initiateConnection(targetPeerId) {
    if (this.peerConnections.has(targetPeerId)) return;

    // 1. RTCPeerConnectionの作成
    const pc = await this.createPeerConnection(targetPeerId);

    try {
        // 2. Offer SDPの作成
        const offer = await pc.createOffer();
        // 3. ローカルSDP（LocalDescription）の設定
        await pc.setLocalDescription(offer);

        // 4. シグナリングサーバー経由で対象視聴者へOfferを送信
        const message = {
            type: 'offer',
            connection_id: targetPeerId, // 送信先視聴者ID
            sender_id: this.connectionId, // 配信者ID
            data: offer
        };
        this.ws.send(JSON.stringify(message));
    } catch (e) {
        console.error(e);
    }
}
```

### 3.4 ピア接続時のストリーム追加
`createPeerConnection` 内で、取得済みの `localStream` の全トラック（映像・音声）をピア接続に登録します。これにより、視聴者へメディアデータが送信可能になります。

```javascript
if (this.localStream) {
    this.localStream.getTracks().forEach(track => {
        pc.addTrack(track, this.localStream);
    });
}
```

---

## 4. 視聴者（Viewer）の実装仕様

視聴者はパッシブ（受動的）に動作し、配信者から送られてくる SDP Offer をトリガーにWebRTC接続を構築します。

### 4.1 SDP Offer受信時の処理
視聴者は WebSocket を介して配信者からの `offer` を受信すると、以下の手順を実行します。

1. **既存接続の破棄**: 該当の配信者からの古い接続（`RTCPeerConnection`）が存在する場合はクローズします。
2. **PeerConnection作成**: 新しい `RTCPeerConnection` インスタンスを作成します。
3. **リモートSDP（RemoteDescription）の設定**: 受信した SDP Offer を設定します。
   ```javascript
   await pc.setRemoteDescription(new RTCSessionDescription(message.data));
   ```
4. **動的ビデオ要素の作成**: `#videoGrid` 内に、配信者IDに対応する `<video>` タグを動的に生成します。
5. **トラック受信イベントハンドラ登録**: 配信者からの映像/音声トラックが到着した際に、生成した `<video>` 要素にストリームをバインドします。
   ```javascript
   pc.ontrack = (event) => {
       if (event.streams && event.streams[0]) {
           videoElement.srcObject = event.streams[0];
       }
   };
   ```
6. **Answerの生成と送信**: SDP Answer を作成し、ローカルに設定した上で、配信者へ返送します。
   ```javascript
   const answer = await pc.createAnswer();
   await pc.setLocalDescription(answer);
   // WebSocketで配信者へAnswerを送信
   ```

### 4.2 自動接続モード (Auto Connect Mode)
`viewer.html` には、ルームの手動入力接続のほかに「自動接続モード」が搭載されています。

- **自動検出**:
  有効化されると、5秒ごとに定期実行タイマー（`setInterval`）が走り、代表的なデモルームID（`demo`, `test`, `public`）に対して `/api/rooms/{room_id}` の存在確認APIをコールします。
- **自動再接続**:
  WebSocket接続が切断された場合、自動接続モードが有効であれば 3秒後 に自動的にシグナリングサーバーへの再接続を試みます。

---

## 5. ICE 候補 (ICE Candidate) のやり取り

WebRTC接続の接続性（P2P経路）を確保するため、双方のクライアントは以下の処理を行います。

1. **候補の検出**: `RTCPeerConnection` インスタンス上で `onicecandidate` イベントが発火した際、検出された ICE Candidate データをWebSocket経由で対向ピアへ直ちに送信します。
2. **候補の適用**: WebSocketから `ice_candidate` メッセージを受信した際、対応するピアの `RTCPeerConnection` に候補を追加します。
   ```javascript
   await pc.addIceCandidate(new RTCIceCandidate(message.data));
   ```

---

## 6. 切断処理 (Cleanup)

- **明示的な退出/切断**:
  クライアントはWebSocket切断時、またはブラウザ終了時にサーバーによりコネクションマップから自動的に削除され、同じルームの対向ピアに `leave` メッセージが同報されます。
- **リソースの解放**:
  `leave` メッセージを受け取ったピアは、該当する `RTCPeerConnection` をクローズし、関連するDOM要素（視聴者側の`<video>`要素）を削除します。
- **コネクションステート変化監視**:
  各ピア接続の `onconnectionstatechange` イベントを監視し、接続状態が `disconnected`, `failed`, `closed` に遷移した場合にも、JavaScriptの `Map` からインスタンスを除去しメモリリークを防ぎます。

---

## 7. ナビゲーションポータル (Entrance Portal)

`static/index.html` は、Warpサーバーのルートパス (`/`) にアクセスした際に表示されるエントランス画面です。

### 7.1 機能とレイアウト仕様
- **ルート誘導機能**: 配信画面（`sender.html`）および視聴画面（`viewer.html`）へのリンクを配した直感的な2カラム・グリッドカード。
- **LAN内アクセス情報の表示**: 現在アクセスしているURLをベースに、同一ネットワーク内の他デバイス（スマートフォン等）からアクセスする際の手順やURLを案内します。
- **モダンデザイン**: 暗色基調（ダークモード）にネオン調のグラデーション、すりガラス効果（Glassmorphism）、ホバー時の拡大インタラクションなどの演出により、プレミアムで直感的な操作感を提供します。
