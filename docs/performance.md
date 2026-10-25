# 性能の測定と品質契約

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
