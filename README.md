# Lagui

軽量・低遅延のネイティブ GUI ライブラリを開発するための初期実験。
ブラウザ、WebView、HTML、CSS、JavaScript は使用しません。

> **Stability:** 0.1.xは公開APIを固める前のalphaです。破壊的変更はCHANGELOGへ記録し、
> 安定化後はSemVerに従います。新規Widgetでは重複IDを検出する
> `register_semantic_unique`または`widgets::register`を使用してください。

主言語には **Rust** を選定しました。代替言語との比較は
[言語選定記録](docs/language-decision.md) を参照してください。

Rust の描画コアと、操作できるネイティブのスタジオデモを実装しています。
**主実装は winit + wgpu**。単体の GUI ライブラリとして開発しています。
wgpu はネイティブ GPU API を呼び出します。WGSL は GPU シェーダーのソースであり、ブラウザを起動しません。
ブラウザ向け wgpu 機能は無効です。

任意のアプリを構築するための公開部品として、可変長の入力ランタイム、
Flexレイアウト、仮想リスト、TextEdit、ポップアップ/モーダル管理、
アクセシビリティ意味情報、テーマ可能な標準コントロール外観を提供します。
組み込み方は [GUIライブラリ利用ガイド](docs/library-guide.md) を参照してください。

角丸primitiveは`CornerStyle::new(radius, smoothing)`で0〜1のcorner smoothingを指定できます。
`VisualList::fill_styled`へ`DropShadow`を渡すと、offset・spread・Gaussian blurを持つ
解析的なdrop shadowを形状と同じGPU commandで描画します。従来の`fill`と`outline`は
smoothing 0、shadowなしとして互換性を維持します。

コアは `no_std`。文字境界のために no_std の unicode-segmentation を使用します。
既定機能ではヒープ確保せず、`alloc` を有効にすると入力・パネル・タブ・Overlay を動的に拡張します。
比較用として、Objective-C 接続層による macOS / 直接 Metal 版も残しています。
採用理由、サイズ、検証結果は [バックエンド選定記録](docs/backend-decision.md) を参照してください。

## 起動

Rustup と Xcode Command Line Tools が必要です。Rust 1.92.0 を固定しています。

```sh
cargo run --release --features portable --bin studio-demo
```

- レイヤーまたはノードをクリックして選択。ノードをドラッグして移動。
- 空のキャンバスをドラッグしてパン、ホイールまたは `+` / `-` でズーム。
- 左右の境界とタイムライン上端をドラッグしてペインをリサイズ。
- インスペクターのOpacity、Enabled、Snap、Play、タイムラインの再生ヘッドを操作。
- 中央ペインをPREVIEWへ切り替えると、`GpuCanvas`内の深度付き3D描画を表示。
- `3D Window`で同じGPUプレビューを独立した第2ウィンドウに表示。
- レイヤー検索欄への文字入力、Tab / Shift+Tabでフォーカス移動、Enterで実行。

簡単な文字・スライダーデモも残しています。

```sh
cargo run --release --features portable --bin portable-demo
```

macOS は Metal、Windows は DX12、Linux は Vulkan を指定します。実機検証は macOS のみです。
wgpu 版の文字は **Windfoil方式の解析的box-filterシェーダー**で描画します。ラベルの英字と日本語を GPU が直接描きます。
既定では OS の日本語フォントを読み込みます。別のフォントを使う場合：

```sh
LAGUI_FONT=/path/to/font.ttf cargo run --release --features portable --bin portable-demo
```

TTF/OTF/TTC 対応。TTC のフェイス番号は `LAGUI_FONT_INDEX` で指定できます（既定0）。
デモは日本語を含むため、日本語グリフのあるフォントが必要です。OSフォントは同梱・再配布しません。

変更時だけ描画します。wgpu には最大フレーム遅延1を要求しますが、これは厳密な GPU 待ち時間の保証ではありません。
低遅延モード（AutoNoVsync）を既定とし、非対応環境では対応する方式へフォールバックします。
表示タイミングやティアリングの挙動は環境に依存します。
表示同期ありで比較する場合：

```sh
LAGUI_VSYNC=1 cargo run --release --features portable --bin portable-demo
```

比較用の直接 Metal 版（macOSのみ。ラベルは AppKit が描画）：

```sh
cargo run --release --features native --bin native-demo
```

直接 Metal 版は通常経路で同時投入する GPU コマンドバッファを最大1つに制限します。

## 検証

```sh
bash scripts/check.sh
bash scripts/check.sh gpu
cargo run --release --example draw_list_bench
cargo run --release --features portable --bin studio-demo -- --smoke
cargo run --release --features portable --bin portable-demo -- --smoke
cargo run --release --features native --bin native-demo -- --smoke
```

スタジオの `--smoke` は選択・ノードドラッグ・Opacity変更を合成入力で通し、
角丸、境界、クリップ、Windfoil文字をGPUで1280×768へ描画します。ピクセルを読み戻して文字出力を検査し、
`build/studio-smoke.ppm` を保存します。

wgpu 版の `--smoke` はウィンドウへの1フレーム描画に加え、Rust の合成入力による状態変更と
GPU 出力ピクセルを検証し、文字を含む `build/wgpu-smoke.ppm` を保存します。
日本語・英字の出力に加え、独自のテストフォントで12px/96px、1倍/2倍の輪郭・内側の空白・AAを検証します。

直接 Metal 版の `--smoke` はウィンドウを開き、合成マウスイベントを AppKit へ送り、Rust の状態変更を経て
Metal の出力ピクセルを検証して終了します。GPU 出力を `build/native-smoke.png` に保存します。
この画像に AppKit のラベルは含まれません。実行にはログイン済みの macOS デスクトップと Metal GPU が必要です。
合成イベントの値は実際の入力機器の遅延ではありません。

## 実装範囲と計測の意味

矩形と角丸矩形のバッチ描画、境界線、論理座標クリップ、Retina対応、リサイズ、
ボタン、トグル、スライダー、スクロール、ポインターキャプチャ、キーボードフォーカスを実装しています。
さらに選択・IME合成・undo/redo、複数行・単語移動を持つ共通TextEdit/TextEditor、Flex、Grid、仮想化、Overlay stack、
意味情報の列挙APIを実装しています。クリップボードとOSアクセシビリティへの接続は
`platform`モジュールのアダプター契約を通してホスト側が担当します。
スタジオデモでは可変ドック、ノードグラフ、インスペクター、タイムラインを同時に動かします。
描画リストは呼出側の固定容量バッファを借用し、容量超過をエラーで返します。
GPUインスタンスは必要量まで動的に拡張し、GPU deviceとアドレス表現の実際の上限でエラーを返します。
コンストラクターの容量は初期reservationです。色絵文字のAtlasも拡張し、既存のlayerと画像を保持します。

**Windfoilの公開アルゴリズムをWGSLへ移植し、HarfRustで文字組み、ttf-parserで輪郭取得を行います。**
TrueTypeの二次曲線はそのまま、CFFの三次曲線は誤差を制限して二次曲線へ変換します。
輪郭と計算済みの文字列を `TextCache` で再利用するため、サイズ変更時に再ラスタライズ・再シェーピングは行いません。
デモの初期reservationは上限ではありません。仕様とAPIは [文字描画](docs/text-rendering.md) を参照してください。

高度なデモから判明した実装済み範囲と残件は
[GUI検証記録](docs/gui-validation.md) にまとめています。
Slug/Windfoilの同一画面ベンチとOSアクセシビリティの実装アダプターは未完成です。
段落の双方向処理、折り返し、font fallback、hit testingには、共有FontSystemを使う
`text::paragraph::Paragraph`を公開しています。言語別の禁則・組版品質は実機検証を継続します。
Studio demoではgrapheme単位で連続runへ分割し、Windfoilが扱えないrunだけをシステムフォントの
Glyphon/Swash経路へ送ります。通常文字はWindfoilのまま、カラー絵文字とBiDi fallbackだけをアトラス描画します。
順序付きフォントスタックによるフォールバック選択、IME preedit状態、アクセシビリティ出力契約も実装済みです。
Graphyは共通エディタとrendererの利用例です。共通の修正はLaguiに集約します。
Windows/Linux のコードは winit/wgpu を通じて用意しましたが、ビルド・実機動作は未検証です。

入力計測は、反映した最新のイベントをアプリが受信してから GPU へ投入するまでです。
wgpu 版では `Queue::submit`、直接 Metal 版では `commit` の呼出完了を終点とします。
矩形命令の構築、文字インスタンスの準備・転送（wgpu版）、surface/drawable の取得待ちを個別に記録します。初期化、OS内のイベント滞留、
AppKit のラベル描画、GPU完了、実際の発光を含む完全な遅延ではありません。
イベントがまとめられた場合、古いイベントからの待ち時間はこの値に含みません。
1ms未満や世界最軽量を達成したと主張する段階ではありません。

CPU マイクロベンチマークは 4,096 個の矩形命令の構築のみを計測します。
比較用の初期 C++ ベンチマークは次で実行できます。

```sh
make bench-cpp
```

仮想リストのレイアウト、意味情報登録、ヒットテストを含むCPUフレームベンチは次で実行できます。

```sh
make bench-gui
```

設計と段階的な実装方針は [docs/design.md](docs/design.md) を参照してください。

## アプリケーションとの境界

`renderer-wgpu` 機能で `lagui::gpu::{Renderer, VisualRenderer}` を有効にできます。
`VisualRenderer::new(&device, format, capacity)` にホストのdeviceを渡し、
`prepare(&queue, visuals, viewport, scale_factor)` と `draw(&mut render_pass)` で既存の描画パスへ組み込みます。
ウィンドウ作成やpresentはホストが管理します。
同じ device の queue を使い、各 prepare のデータを encode/submit してから次の prepare を行ってください。
容量は初期reservationで、必要量まで拡張します。GPU上限を超える場合はエラーで返します。

アプリ独自の3D・動画・チャート描画をGUI内へ埋め込む場合は
`lagui::gpu::GpuCanvas`を使います。論理座標のbounds/clipから物理pixelのviewportと
scissorを生成し、ホスト所有のwgpu render passへ適用できます。ウィンドウとSurfaceは
ホスト所有なので、同じイベントループで複数ウィンドウを作成できます。

## 性能と品質を確認する入口

```sh
cargo run --release --locked --features alloc --example performance
bash scripts/check.sh gpu
```

performanceは32/512/10,000領域の登録、疎なID、重複領域、hoverのp50/p95/p99と
ウォーム時の確保回数をJSON Linesで出力し、入力・登録の追加確保ゼロを検査します。
これはCPU/allocatorの検査で、GPU実行時間や入力から発光までの遅延ではありません。
[計測条件と結果](docs/performance.md)、[利用ガイド](docs/library-guide.md)、
[API変更](CHANGELOG.md)を参照してください。
