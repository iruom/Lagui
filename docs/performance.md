# 性能の測定と品質契約

## 日本語500ラベルの積分再利用（2026-10-29）

Windows 11 / Core i9-13900K / RTX 3060 / NVIDIA 616.56 / Vulkan / Rust 1.92.0 Release。
Graphyの `scripts/ui-compare` から、1024×768・BGRA8 sRGB・IMMEDIATE・非表示windowで、
500個の矩形と12pxの「図形編集」（Meiryo TTC face 0）を保持描画した。
変更前は `ffd23a3` のソースを別checkoutでビルド。各runはwarm-up 600、計測500 frame。
変更前・変更後・Slugia・実験用統合版の順序を回転して3回測り、各runのpercentileの中央値を示す。
GPU clockは固定しておらず、NVML記録は主に1995 MHz（起動直後1792〜2017 MHz）。
以前の780 MHz条件の比較結果と、この絶対時間を直接比べない。

| 保持描画 | GPU p50 ms | GPU p95 ms | GPU p99 ms | wall p50 ms |
|---|---:|---:|---:|---:|
| Lagui 分割・変更前 | 0.524288 | 0.537600 | 0.544768 | 1.0684 |
| Lagui 分割・変更後 | 0.396288 | 0.406528 | 0.411648 | 0.9399 |
| Lagui 統合・変更後（実験用） | 0.463808 | 0.471040 | 0.475136 | 0.5626 |
| Slugia `91370b7` 保持 | 0.349824 | 0.354304 | 0.357536 | 0.4137 |

分割版のGPU時間は24.4%短縮、wallは12.0%短縮。SlugiaのGPU時間にはまだ13.3%届かない
（変更後Laguiの時間がSlugiaの1.133倍）。AAの既定値は両者で異なる。
CPU encode/submitのp50は分割版で0.4678→0.4709 msと改善していない。
これは定常描画の局所的なGPU改善であり、入力遅延・初回構築・全面優位の主張ではない。

原因は、小さな文字の1pixelが複数バンドを覆い、同じ曲線の二次方程式と積分を繰り返すことだった。
文字向けの同一shader定義の実行版で、pixelと最初に重なる行だけが曲線全体のbox積分を担当する。
複数行では輪郭全体で同じ左右方向を使い、別の行の部分積分を混ぜない。
直線・垂直曲線は正確な簡略積分を使う。文字列や日本語への特例、ラスタ化、近似AAは追加しない。
atlas結合時には元曲線の所属行も再配置する。通常vector版は従来の実行版を使い、
同じ128曲線instanceの対照測定はGPU p50が0.088064→0.088064 msだった。

変更前後の5シーン（日本語、英字混在、高密度英字、曲線、zoom）はRGBA readbackの最大差0。
独立したf64のpolygon/pixel clippingで、穴・重複した輪郭・斜線・負scale・clipを検証し、
色と半透明のcoverage transferもGPUテストで確認する。
GPU曲線構造体は従来のpaddingを使い32 bytesのまま。小さな行にも左向きのindexを持たせるため、
それらの行のindex storageと、CPU側の曲線の所属行metadataは増える。

28 run / 14,000計測frameのraw、RGBA、実行コマンド、binary SHA-256:
`C:/Users/mouri/.codex/tmp/lagui-labels-final-verified/`。
clock記録: `C:/Users/mouri/.codex/tmp/lagui-labels-final-verified-telemetry.jsonl`。
比較プログラムはGraphyの `scripts/ui-compare/`、再現時は変更前ソースもReleaseでビルドする。

検証: `bash scripts/check.sh` はfmt・core・alloc・text・portable・doctest・docs・performance・packageが通過。
Clippyだけ既存の `src/gpu/emoji.rs:472` の `collapsible_if` で失敗
（`build/checks/run-j53ruZ20/`）。`bash scripts/check.sh gpu` は追加テストを含め通過
（`build/checks/run-q3JkIUjC/`）。smokeはbasic通過、studioのpanel色が
expected `[49,56,66]` / actual `[56,56,56]` で失敗（`build/checks/run-7n1Blzc7/`）。
Clippyとstudioは変更前 `ffd23a3` のソースでも同じ失敗を再現した。
そのbaselineログはGraphyの `build/checks/labels-baseline-clippy.log` と
`build/checks/labels-baseline-studio.log` に保存した。

## UI登録・入力CPU（2026-10-11）

2026-10-11、Apple M3 Pro、macOS/aarch64、Rust 1.92.0で確認。
比較対象はGraphy commit 5b02959に入っていたalloc UI登録実装。
同じ350回のループ（warm-up 50、計測300）、rustc -O、同じ入力、同じ端末で比較した。
各frameで安定IDをunique登録し、hoverとend_frameを実行する。
全領域は同じ100×100の矩形なので、hitは最前面の領域に当たる。
計測は**UI登録・入力CPUだけ**で、レイアウト、文字、GPU、表示待ちは含まない。

| 領域数 | 変更前 p50 µs | 変更後 p50 µs | 変更前 p99 µs | 変更後 p99 µs |
|---:|---:|---:|---:|---:|
| 32 | 0.583 | 0.292 | 0.709 | 0.417 |
| 512 | 16.000 | 4.875 | 23.750 | 6.250 |
| 10,000 | 698.750 | 110.667 | 756.375 | 140.333 |

10,000領域の同条件比較でp50は約6.3倍、p99は約5.4倍改善。
BTreeMapの破棄・再構築を、generation付きの再利用可能なID索引に変更した。
これはその処理の改善率で、GUI全体や他ライブラリに対する速度差ではない。

## 再現と退行検査

```sh
cargo run --release --locked --features alloc --example performance
bash scripts/check.sh gpu
```

performanceはJSON Linesを出力する。32/512/10,000領域、連続ID/疎なID、
重複領域、hoverのp50/p95/p99/最大値とOS/arch/sample数を記録する。
各ケースのwarm-up後はregistration/inputで追加確保しないことをallocatorで検査する。
初回の領域/index拡張、文字列生成、wgpu/OS内の確保はこの保証に含まない。
非常に短いhoverではclockの分解能により0 µsとなることがあり、処理時間ゼロを意味しない。

報告にはcommit、rustc、OS、CPU/GPU、電源状態、画面サイズ、DPI、可視件数、
フォント、表示同期、warm/coldを添える。性能しきい値は安定した専用端末で判断し、
shared CIの揺れだけで退行と判定しない。

## 品質の合格条件

- Unicode 16公式GraphemeBreakTestの全ケースで前後のカーソル位置が一致する。
- 容量不足のIME確定で本文・選択・preedit・Undo/Redoを失わない。
- 変更部分のUndoを可変長で保持し、組み合わせたgraphemeを壊さない。
- モーダルは背景のpointerとkeyboard入力を遮断し、disabledはclick-throughしない。
- 削除されたsplit IDは別の分割を指さず、残ったIDはNodeの移動後も有効。
- GPU拡張でpipeline/atlas/painter orderを保持し、geometryを切り捨てない。
- Shader回帰は独立した参照形状と実GPUのreadbackで確認する。
- GPU adapterがない検証はSKIPを記録し、gpuモードでは失敗にする。

## 継続する評価

クロスライブラリ比較、GPU timestamp、待機電力、起動/RSS/VRAM、物理入力から発光までの
遅延、Windows/Linuxの実機、スクリーンリーダーとのOS adapterは別途必要。
Paragraphは既存の成熟した文字組み経路を共通APIに接続した段階で、すべての言語の
禁則・組版・視覚キャレット操作が検証済みという意味ではない。
世界最速や全用途10/10を主張するためのデータはまだない。

改善の実装設計と順序は [描画性能の改善設計](performance-roadmap.md) にまとめた。
