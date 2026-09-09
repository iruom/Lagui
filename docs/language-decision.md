# 言語選定：Rust

日付：2026-09-08。対象：Lagui の CPU 側コアと、今後の Graphy への統合。
決定：Rust を主言語にする。GPU シェーダーの言語と描画 API は別に選定する。
これは設計上の選択であり、言語別の実測速度ランキングではない。

## 判断に使った Graphy の現状

隣接する `../Graphy-26.8` のコードとドキュメントを確認した。
別の後継リポジトリの存在や今後の全面再実装は仮定していない。

- `Cargo.toml`：Rust edition 2024。wgpu 30.0.1、winit 0.30.13、cosmic-text 0.19、taffy 0.13。
- `README.md` / `docs/cross-platform-build.md`：macOS と Windows が対象。
- `src/ui/component/display_list.rs`：型付き描画リスト、クリップ、入力領域。
- `docs/gui-foundation.md`：フォーカス、文字編集、IME、クリップボードの基盤。
- `src/render/mod.rs`：描画側とエンジン側で wgpu device/queue を共有する構成。
- `src/render/text.rs`：cosmic-text/Swash と 4096×4096 R8 のラスタグリフアトラス。
- `crates/graphy-vector-engine`：アプリから独立した Rust のベクトル描画エンジン。
- `Cargo.toml` の release 設定：ビルド反復時間と Windows のコンパイラ／リンカ問題に配慮。
- `AGENTS.md`：互換性維持は不要。既存 API を温存するために Rust を選ぶのではない。

ソース確認のみ。Graphy 本体は変更・実行しておらず、現在の性能の優劣は測定していない。

## 選定の優先順位

1. メモリ配置・確保・同期を制御でき、通常処理の低遅延化を妨げない。
2. Mac/Windows の GPU、IME、文字組み、アクセシビリティへ接続できる。
3. Graphy のデータや GPU 資源を余計なコピーなしで扱う設計が容易。
4. 長期にわたり、所有権・並行処理・ツールチェーンを保守できる。
5. 小さい配布物と、開発中の短いビルド反復を両立できる。

小ささは同等機能の成果物で比較する。文字描画や OS 連携を外した最小実行ファイルを
完成した GUI と比較しない。以下の評価はこの用途に対する判断であり、一般的な言語の順位ではない。

## 候補比較

| 候補 | 強み | 今回の判断 |
| --- | --- | --- |
| Rust | GC 不要、連続メモリ、所有権チェック、Graphy と型・ビルドを共有 | 採用。低遅延化と長期開発のバランスがよい |
| C++ | RAII、低水準制御、既存ネイティブ SDK との親和性 | 最終候補。Rust より速いという根拠がなく、言語境界と保守規律が増える |
| Zig | 明示的アロケータ、comptime、C 連携、クロスコンパイル | 最終候補。制御性は魅力的だが、言語・標準ライブラリ・ビルドの変更追従が増える |
| C | 小さい C ABI ライブラリを作りやすく、依存を抑えやすい | 他言語向け境界に有力。大きな GUI の内部実装では型と寿命の手動管理が増える |
| Odin | データ指向、アロケータ、グラフィックス向けの設計 | 有力な代案。新しいツールチェーンと Graphy 連携を導入するだけの優位を確認できない |
| C3 | C 連携、手動メモリ管理、temporary allocator | 研究候補。Graphy の主基盤として採る積極的根拠が不足 |
| Swift | Apple API との連携、値型、ARC | macOS 専用なら強い候補。Windows も含む共通 GUI コアには選ばない |
| Objective-C / Objective-C++ | Apple の OS 境界を直接記述できる | 必要に応じた小さい OS 接続層の候補 |
| D / BetterC | BetterC で D ランタイムへの依存を避けられる | 制限された機能・ライブラリと追加ビルドを引き受ける利点が小さい |
| Nim | C/C++ 出力、ARC/ORC、表現力 | 実装可能だが、参照カウントや生成コードを含めた性能調査の層が増える |
| C# / NativeAOT | AOT 配布と強い開発ツール | AOT と GC 排除は別。最小コアにランタイムを追加する優先度が低い |
| Go | 単純な言語とビルド、並行処理 | GC とランタイムを前提にした設計を、このライブラリの基準にはしない |
| Java / Kotlin、Dart | アプリ開発の生産性 | GC／ランタイムやネイティブ境界の調整より、直接制御できる候補を優先 |
| Python / Lua、Julia | 拡張・自動化・数値処理 | 補助用途。最小 GUI コアを担わせる積極的理由がない |
| Assembly | 個別の命令列を直接制御 | 測定で必要性が出た狭い処理に限定。移植可能な GUI 全体の主言語にはしない |

後半の候補は要件で一次選別したもの。全候補の同等実装・コンパイラ比較は行っていない。
GC を持つ言語で高速 GUI が作れない、という意味ではない。

## Rust を選ぶ理由と代償

Rust の所有権・借用の検査は主にコンパイル時に行われ、GC を必要としない。
連続配列、固定容量バッファ、arena と ID で GUI を構成できる。
UI ツリーを `Rc<RefCell<Node>>` の接続網にする必要はない。
安全な API の内部に OS/GPU 向け unsafe を限定できる点も長期保守に有利と判断する。

ただし Rust なら自動的に軽くなるわけではない。
境界チェック、Arc の更新、clone、ヒープ確保、ロック、Drop の連鎖、
ジェネリクスの具体化によるコード増加、コンパイル時間は設計・計測対象である。
依存を増やしたままでは配布物も小さくならない。
安全チェックを一律に外すのでなく、配列走査・所有構造・データ配置から改善する。

C++ や Zig との FFI 呼び出し自体が必ず高コストということではない。
問題は、複雑な UI 型・文字列・コールバック・GPU ハンドルについて
寿命、解放側、スレッド、エラー伝播を別途取り決める作業である。
ゼロコピーも可能だが、その契約を保守する必要がある。

Zig 0.16 の公開リリースノートでは I/O、C import、型生成などに大きな変更がある。
バージョン固定は可能だが、将来の OS/SDK 対応で更新するときの追従費用は残る。
Zig に速度上の欠陥があるから除外するのではなく、Graphy に対する総合的な便益で判断する。

## 文字描画・GPU との関係

Slug/Windfoil の曲線評価は GPU シェーダーで行う。
CPU 側の言語を Zig に変えるだけで、その GPU 処理が高速化するとはいえない。
CPU 側ではグリフ抽出、シェーピング、バンド構築、キャッシュ、アップロードを最適化する。
同じシェーダーと同じ入力を用いて初めて、CPU 側の実装差を比較できる。

Rust からも Apple フレームワークへ接続できる。Rust 採用は wgpu 採用と同義ではない。
言語決定後、利用者から winit/wgpu を含めた判断を委ねられ、主実装には両者を採用した。
ブラウザ/WebView を使わず、ネイティブ GPU API で描画する。シェーダーは WGSL。
wgpu のブラウザ向け機能は無効にする。直接 Metal/MSL 版は比較用に保持する。

既存 Graphy の wgpu はネイティブでも Metal/Vulkan/DX12 を利用できるが、
既存 Graphy との GPU 資源共有と OS 対応を重視して採用した。
直接 Metal 等へ移る場合、同じ Rust でも GPU 資源共有は自動的には解決しない。
Graphy の描画 API 境界・同期・資源寿命を含めて設計する。
二つの device を安易に作り、画像を CPU 経由で受け渡す構成を避ける。

## 実装への反映

- 主言語は stable Rust。ツールチェーンを固定し、更新を管理する。
- コアは GPU/OS/文字組みエンジンに依存しない小さいモジュールにする。
- 描画リストは借用バッファまたは事前確保した連続領域へ書き込む。
- UI の状態は ID と所有ストレージで管理。非同期 executor をコアの必須依存にしない。
- Graphy は Rust API を直接使う。将来の他言語向けには必要な範囲だけ C ABI を用意する。
- C ABI には不透明ハンドルと明示したデータ構造を使い、Rust の通常の型配置や ABI を公開契約にしない。
- 開発用と配布用のビルド設定を分け、LTO 等はサイズ・速度・ビルド時間を測って選ぶ。
- 前ターンの C++ 描画リストは比較用の小実験として保持。本採用の根拠にはしない。

この決定により C++ の初期設計を置き換える。
2026-09-08 の実装で Rust 描画コアと macOS/Metal デモを追加した。
OS 接続は Objective-C の C ABI アダプタ、UI の状態と描画命令の構築は Rust が担当する。

## 再検討する条件

実用シーンの p99 や配布サイズが目標を満たさず、原因が依存・アルゴリズム・GPU 待ちではなく
言語／コンパイラにあると切り分けられた場合に限り、同等の C++/Zig 実装と比較する。
その他、C ABI での小さい組込み配布が最優先になった場合や、Graphy が Apple 専用になる場合は再評価する。
Hello World のサイズや矩形ループだけで主言語を変更しない。

## 一次資料

- [Rust の設計上の特徴](https://rust-lang.org/)
- [Rust 型配置と C representation](https://doc.rust-lang.org/reference/type-layout.html)
- [Zig 0.16 リリースノート](https://ziglang.org/download/0.16.0/release-notes.html)
- [Odin のメモリ管理と allocator](https://odin-lang.org/docs/overview/)
- [C++ Core Guidelines](https://isocpp.github.io/CppCoreGuidelines/CppCoreGuidelines)
- [C3 メモリ管理](https://c3-lang.org/language-common/memory/)
- [Swift C++ 相互運用と対象プラットフォーム](https://www.swift.org/documentation/cxx-interop/status/)
- [D BetterC の機能と制限](https://dlang.org/spec/betterc.html)
- [Nim ARC/ORC](https://nim-lang.org/docs/destructors.html)
- [.NET Native AOT](https://learn.microsoft.com/en-us/dotnet/core/deploying/native-aot/)
- [Go GC ガイド](https://go.dev/doc/gc-guide)
- [Rust から Apple framework への接続](https://github.com/madsmtm/objc2)
- [wgpu のネイティブバックエンド](https://github.com/gfx-rs/wgpu)
- [Rustybuzz](https://github.com/harfbuzz/rustybuzz)
- [Slug](https://github.com/EricLengyel/Slug)
- [Windfoil](https://github.com/texel-org/windfoil)
