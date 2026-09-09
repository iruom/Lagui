# 描画・ウィンドウ基盤の選定

更新：現在は単体ライブラリを対象とし、Graphy組み込みは利用者の指示で対象外。
以下は基盤選定時の記録。後続でSlug方式の文字描画を実装した（[仕様](text-rendering.md)）。
下記サイズは文字描画追加前の値。

2026-09-08：主実装は **winit 0.30.13 + wgpu 30.0.1** とする。
利用者から両ライブラリを含めた判断を委ねられたため、既存 Graphy との統合、
Windows/macOS 対応、低遅延化の余地を合わせて選定した。

## 役割

- コア：Rust。既定機能では `no_std`、外部依存なし。状態と描画命令を扱う。
- winit：ネイティブウィンドウとイベント。レイアウトや文字描画は提供しない。
- wgpu：ネイティブ GPU API へ描画命令を送る。ブラウザや WebView は使用しない。
- Slug/Windfoil：今後実装する文字描画方式。winit/wgpu の採用とは独立して比較する。

macOS は Metal、Windows は DX12、Linux は Vulkan に限定する。
wgpu の default features を無効にし、native API と WGSL の機能だけを選ぶ。
Cargo.lock に他ターゲット用のパッケージがあっても、それらをネイティブ実行ファイルへ全て組み込むわけではない。

`renderer-wgpu` は winit に依存しない。既存の device、queue、render pass を受け取り、
ウィンドウ、surface、present はホストが所有する。Graphy 本体の変更はまだしていない。
同一デバイスの資源を扱える入口を実装した段階であり、統合済みと主張しない。

## 追加コスト

wgpu は API 検証、資源追跡、シェーダー変換などを担うため、直接 API より薄いとは限らない。
そのため直接 Metal/MSL のデモを比較用に保持する。
表示 API の変更だけで、OS コンポジタやディスプレイの待ちがなくなるわけではない。

手元の Apple M3 Pro / macOS / Rust 1.92.0、release、thin LTO、strip 後の観測：

| デモ | 実行ファイルサイズ |
| --- | ---: |
| winit + wgpu（文字追加前） | 3,784,512 bytes |
| スタジオデモ（Slug文字込み、2026-09-09） | 4,513,120 bytes |
| AppKit 接続 + 直接 Metal | 371,760 bytes |

OS フレームワーク、プロセス常駐メモリ、GPU メモリを含まない。
直接Metal版の文字はAppKitラベルであり、スタジオのSlug文字とは実装が異なるため、
完成した同等GUIの比較ではない。実行ファイルサイズだけで常駐メモリや描画速度も判断できない。

## 検証結果

- コアの4テスト：容量超過・再利用、境界のヒットテスト、ボタンの押下／解放、スライダーの解除を確認。
- 両バックエンドを含む Clippy：警告なし。
- 直接 Metal：AppKit 経由の合成マウス入力、ウィンドウへの投入、GPU ピクセルを検証。安定性修正後3回通過。
- wgpu：ウィンドウに1フレームを描画し、合成入力で変更した状態のオフスクリーン GPU ピクセルを検証。
- いずれも速度の達成判定に使える長時間の入力ベンチマークは未実施。
- Slug方式は実装済み。Windows/Linux実機検証、Windfoil比較、IME変換中表示、アクセシビリティは今後の作業。

描画先取得の待ちが1msを超えるケースを実測しており、1ms未満の達成は主張しない。
次の性能調査は、実際のドラッグで入力サンプルを集め、描画先取得・イベント待ち・GPU 実行を分離する。

## 一次資料

- [winit](https://github.com/rust-windowing/winit)
- [wgpu](https://github.com/gfx-rs/wgpu)
- [Metal の小さいバッファ転送](https://developer.apple.com/library/archive/documentation/3DDrawing/Conceptual/MTLBestPracticesGuide/BufferBindings.html)
