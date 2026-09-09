# 文字描画

単体のwgpu版へ、Windfoil方式のアウトライン文字描画を実装した。
Graphy組み込みは行わない。旧Slugシェーダーは比較資料として残している。

## 処理の分担

1. `Font::new(bytes, face_index)`：呼出側が用意したTTF/OTF/TTCを解析。
2. `Font::shape(text)`：HarfRustでグリフID・位置・送りを計算。結果はサイズに依存しないem単位。
3. 未登録のグリフだけttf-parserで輪郭を取得し、xy単調曲線へ分割して水平行バンドを生成。
4. `TextRenderer::sync_font`：新しいグリフが増えたときだけGPUの輪郭バッファを更新。
5. `prepare`：再利用したTextRunから位置・サイズ・色のインスタンスを作成して転送。
6. `draw`：Windfoilのwinding integralによるpixel box面積カバレッジをフラグメントシェーダーで実行。

文字の画像アトラスやSDFは使用しない。GPUのバッファは輪郭とバンド構造を格納する。
TrueTypeの二次Bézierはそのまま扱う。CFFの三次Bézierは適応分割で二次曲線へ近似する。
近似の制御点誤差上限は1/8192 em。画面上の誤差上限は文字サイズ×DPI倍率/8192 pixel。
極端な拡大でも三次曲線が数学的に完全一致する、とは主張しない。

キャッシュ済みのTextRunは何度でも異なるサイズで描画できる。
通常の`prepare`では文字組み、フォント解析、自前のCPUバッファ拡張は行わない。
wgpu内部の転送用確保までゼロと保証するものではない。
`TextCache`は文字列をキーにこのTextRunを保持し、同じラベルの再シェーピングを防ぐ。

## API

`text`機能で公開する（`portable`では有効）。概略：

```rust,ignore
let mut font = Font::new(font_bytes, 0)?;
let title = font.shape("ベジェ曲線から、文字を描く。")?;
let mut renderer = TextRenderer::new(&device, surface_format, 1024)?;
renderer.sync_font(&device, &font);

renderer.prepare(&queue, &[TextSection {
    run: &title,
    baseline: [40.0, 80.0],
    size: 24.0,
    color: [1.0, 1.0, 1.0, 1.0],
    clip: None,
}], [window_width, window_height], scale_factor)?;
renderer.draw(&mut render_pass);
```

`baseline`と`size`は論理pixel。色は線形・premultiplied RGBA。
`clip`には論理pixelの切り抜き矩形を指定でき、`None`はviewport全体になる。
同じdeviceのqueue/render passを使い、そのバッチをsubmitしてから次のprepareを行う。
追加の文字列をshapeして新グリフが入った場合は、次のprepare前にsync_fontを呼ぶ。
古いアトラスや別フォントのTextRun、容量超過、不正な座標はエラーで返す。
異なるフォントは別のTextRendererを使うか、明示的にsync_fontで切り替える。

## デモ

`make run`で複数ペイン、ノード、インスペクター、タイムラインを持つスタジオを描画する。
`make run-basic`では見出し、操作説明、日本語サンプルを描画する。
スライダーで下部の日本語を14〜38論理pixelに拡大縮小できる。
OSの日本語フォントを既定で読む。任意のフォントはLAGUI_FONT、TTCの番号はLAGUI_FONT_INDEXで指定する。
フォント探索とファイル読込はデモ側にあり、ライブラリ自身はOSフォントを探さない。

Apple M3 Pro / macOS / ヒラギノ角ゴシックW3 face 0で、78種のグリフ、
8,709本のバンド複製済み単調二次曲線、292,468 bytesのGPUアトラス内容を確認した。
これはGPU資源全体の割当量やプロセス常駐メモリではない。
2026-09-09のreleaseスモークでは、フォント読込・初期文字組み・文字パイプライン作成は単発で約5.3msだった。
ウォーム時の入力遅延や最速性は別途計測が必要。

## 検証

- CPU：全12テスト。描画コアに加え、三次曲線分割、バンド順序、合字、結合文字、空白、キャッシュ、欠損グリフ。
- GPU：実フォントの日本語と英字で、背景・文字・中間カバレッジが出ることを確認。
- GPU：独自テストフォントの12px/96pxを1倍/2倍で描き、ストローク、内側の穴、外側、AAを確認。
- GPU API：未同期アトラス、フォントIDの不一致、インスタンス容量超過を確認。
- `--smoke`は全文字入り画像をbuild/wgpu-smoke.ppmへ保存する。

## 現時点の制限

- 各TextRunは一つのフォント・方向・scriptとして扱う。段落の双方向処理、script分割、自動改行、フォントfallbackは未実装。
- 欠損グリフはエラーで返す。カラー絵文字・SVG／bitmap専用グリフは未対応。
- ヒンティング、LCDサブピクセルAA、可変フォントの軸指定、回転／遠近変換、テキスト編集・IMEは未実装。
- グレースケールの解析的box-filter AA、nonzero windingを使用。極小グリフには行ごとのink profileを使う。小サイズの見た目はフォントに依存する。
- インスタンス容量は生成時に指定（最大16,384）。文字サイズは有限かつ正である必要があるが、固定の拡大縮小上限は設けない。
- Windows/Linuxは未検証。直接Metal比較版の文字は従来のAppKitラベルのまま。

## 出典

Windfoilの帰属とライセンス情報は[third_party/windfoil](../third_party/windfoil/)に保持。
計算式とシェーダー構造は[texel-org/windfoil](https://github.com/texel-org/windfoil)のApache-2.0公開実装をRust/wgpu用に移植した。
旧Slug参照コードの著作権表示とMITライセンスは[third_party/slug](../third_party/slug/)に保持。
文字組みは[HarfRust](https://github.com/harfbuzz/harfrust)、輪郭解析は[ttf-parser](https://github.com/harfbuzz/ttf-parser)。
