# Lagui ライブラリ利用ガイド

Lagui は完成形の画面構成を強制しない。アプリケーションが状態、文字列、ウィンドウ、
GPU device、イベントループを所有し、ライブラリは小さく組み合わせ可能な部品を提供する。
そのため、エディター、ツール、メディアアプリ、通常のフォームを同じコアで構築できる。

## 公開モジュール

| モジュール | 用途 |
|---|---|
| `ui` | ヒットテスト、hover/active/focus、ポインターキャプチャ、意味情報 |
| `layout` | Row/Column型Flex配置、余白、仮想リストの可視範囲 |
| `panel` | 安定IDによる水平・垂直パネル分割、スプリッター、リサイズ |
| `text_edit` | UTF-8本文、選択、キャレット、IME preedit、cut/paste、undo/redo |
| `overlay` | tooltip、menu、popup、modalの順序と外側クリックdismiss |
| `viewport` | 中央原点の2D座標変換、カーソル固定ズーム、画面pixel単位のパン |
| `widgets` | Themeと操作状態から標準コントロール外観を描画 |
| `visual` | 固定容量の角丸矩形・境界線描画リスト |
| `text` | HarfRust shaping、アウトラインキャッシュ、Windfoil GPU文字描画 |
| `windfoil` | 任意の塗りパス、アイコン、ノード間ベジェ曲線の解析的GPU描画 |
| `gpu` | 既存wgpu render passへの埋め込み |

`alloc`ではUI・Panel・Tab・OverlayのNは初期reservationです。allocatorを使わない
環境ではNが明示的な容量になります。借用したVisualList/DrawListは呼出側bufferの容量を
守り、失敗したWidgetは入力と描画をrollbackします。`alloc`では`VisualList::growable()`または
`VisualList::with_capacity(n)`で所有storageのlistを作れます。これは伸長し、容量超過を返しません。
`clear`後もallocationを保持するため、frameごとに再利用すれば最大frameの後は確保しません。
GPU容量はdevice/addressの実際の上限まで拡張します。

## 最小のフレーム

```rust
use lagui::{Rect, ui::{Role, Semantics, UiRuntime, WidgetId}};

let mut ui = UiRuntime::default();
ui.begin_frame();
ui.register_semantic_unique(
    WidgetId(1),
    Rect::new(20.0, 20.0, 120.0, 36.0),
    Rect::new(0.0, 0.0, 800.0, 600.0),
    true,
    Semantics { role: Role::Button, label: "Save".into(), ..Semantics::default() },
)?;
// VisualListへ背景とラベルをpaintする。
ui.end_frame();
```

OSのpointer eventを`pointer_move/down/up`へ渡す。戻り値の`target`を安定した
`WidgetId`で照合してアプリ状態を変更する。描画時は`hovered`、`active`、`focused`
を`WidgetState`へ渡せば、マウスとキーボードで一貫した外観になる。
新規実装では`register_semantic_unique`または`widgets::register`を使用する。
同じフレームで`WidgetId`を重複登録すると`RegistrationError::DuplicateId`となるため、
一覧では表示indexではなくデータの安定IDからIDを作る。

標準ボタンとスイッチは`widgets::button`、`widgets::toggle_switch`で登録と描画を
一度に行える。これらは事前にUI・描画容量とID重複を検査し、失敗時に片方だけを
追加しない。ラベル文字列のGPU描画はアプリ側が行い、アクセシブルな名前は
`WidgetSpec::semantics`へ設定する。

独自Widgetでは`widgets::WidgetContext::transaction`を使い、その中で`register`、
`fill`、`outline`を呼ぶ。複数領域・複数描画命令を持つWidgetでも、途中の容量不足や
ID重複時には、そのWidgetが追加した内容だけが一括で取り消される。標準Widgetも
同じ`WidgetContext`上の参照実装であり、専用の内部権限は持たない。

## 文字の配置とキャレット

`Font::metrics()`はascender、descender、line gap、cap height、x-heightをem単位で返します。
`FontMetrics::centered_baseline(top, height, size)`は大文字を帯の縦中央に置くbaselineです。
`TextRun::carets()`はgrapheme境界ごとのbyte offsetとx位置（em）を返します。
合字のように1つのclusterが複数のgraphemeを持つ場合は、cluster幅を均等に分けます。
キャレット位置は`caret_x(byte)`、クリック位置からのoffsetは`hit_test(x)`で求めます。
1つの`TextRun`は単一方向のrunです。段落の折り返しと双方向文字は`text::paragraph`を使います。

## テキスト入力

`alloc`の`TextEditor`は可変長の編集sessionです。本文と履歴に固定上限を持たず、
Undoは変更部分だけを保持します。Graphyも同じ実装を利用します。保存済みDocumentと
確定後のUndoはホストのCommandが所有し、このsessionを別の保存正本にしません。

`TextEdit<N, HISTORY>`はallocatorなしの明示的な容量制限を持つ同じ編集アルゴリズムです。
両方でカーソル移動・削除はUnicode grapheme境界を使います。容量不足の確定は
本文・選択・preedit・履歴を変更せずエラーを返します。
プラットフォームのIMEイベントは`set_preedit`と`commit_preedit`へ接続する。

コピーでは`selected_text()`をOS clipboardへ書き、切り取りでは書いた後に`cut()`、
貼り付けではOSから読んだ文字列を`paste()`へ渡す。これによりコアを`no_std`のまま保ち、
macOS、Windows、Linuxそれぞれのclipboard実装をアプリまたはプラットフォーム層で選べる。

## アクセシビリティ

各コントロールを`register_semantic`で登録し、`semantics()`をプラットフォームの
アクセシビリティツリーへ変換する。role、bounds、disabled、selected、expandedを共通化している。
label/valueは`SemanticText`です。static文字列は`"Save".into()`、動的な値は
`String.into()`または共有`Arc<str>.into()`で渡します。frameの読み取り投影として所有し、
削除されたframeの文字列を解放します。exportは`&Semantics`を借用し、
OS adapterで文字列の複製や別のDocumentモデルを要求しません。

## Overlayとモーダル

Overlayは通常コンテンツの後に登録・描画する。`OverlayStack`の順序がpaint順となる。
modalの領域と子Widgetを登録する**前に**`UiRuntime::set_modal_bounds`を設定します。
それ以後の領域だけがpointer/keyboardの入力scopeとなり、背後のfocusとcaptureはend_frameで解除します。
Escape時はtopをcloseし、外側クリックは`pointer_down`でdismissできる。

## 大量データ

一覧全体を登録・描画せず、`virtual_range`が返した半開区間だけを処理する。
アイテムのIDは表示行番号ではなくデータの安定IDから作る。これによりスクロール中も
focus、selection、pointer captureが別の項目へ移動しない。

## 2D Viewportとトラックパッド

`Viewport2D`はノードエディターやキャンバスで共有する中央原点のcamera stateで、
`world_to_screen`、`screen_to_world`、`pan`、`zoom_at`を提供する。`zoom_at`は指定した
画面座標直下のworld座標を維持する。固定のズーム上限・下限はなく、有限かつ正のscaleだけを受け付ける。
backend非依存の`ViewportGesture::{Pan, Zoom}`は`Viewport2D::apply`へ直接渡せる。

プラットフォーム層ではtrackpadの高精度二軸pixel scrollを論理pixelへ変換して`pan`へ、
pinchの相対倍率を`zoom_at`へ渡す。離散的なmouse wheelのline scrollはカーソル基準の
`zoom_at`へ接続する。Retinaなどではphysical pixelのscroll deltaをscale factorで割ってから
渡す。Studio demoがwinit接続のリファレンス実装になっている。

## 新しいWidgetを作る規則

1. 状態はアプリまたは専用state型に置く。
2. 永続状態を座標や配列indexではなく`WidgetId`へ結び付ける。
3. `register_semantic`で入力領域と意味を登録する。
4. `VisualList`と文字rendererへpaint順で命令を追加する。
5. clipを入力、矩形、文字のすべてへ同じように適用する。
6. focus状態を必ず目に見える形で描く。
7. 容量超過を呼出側へ返す。

Studio demoは、分割ペイン、ノード、スライダー、TextEdit、仮想リストを組み合わせた
リファレンス実装として利用できる。

## Windfoilベクター

`windfoil::Path`は直線、quadratic、cubic Bézierを受け付ける。閉じた輪郭は
`Atlas::add_fill`でアイコン等の塗り図形にし、開いたノード接続線は
`Atlas::add_stroke`でround cap/join付きの図形にする。作成した`Shape`はサイズ非依存で、
`Instance`の`origin`と`scale`を変えて再利用できる。atlas更新後に
`Renderer::sync_atlas`、各frameで`prepare`と`draw`を呼ぶ。

```rust
use lagui::windfoil::{Atlas, Instance, Path};

let mut atlas = Atlas::new();
let mut cable = Path::new();
cable.move_to([0.0, 0.0])
    .cubic_to([80.0, 0.0], [120.0, 100.0], [200.0, 100.0]);
let cable = atlas.add_stroke(&cable, 3.0)?;

let instance = Instance {
    shape: cable,
    origin: [40.0, 80.0],
    scale: [1.0, 1.0],
    color: [0.2, 0.6, 1.0, 1.0],
    clip: None,
};
```

`VisualList::outline_aligned`は`StrokeAlignment::{Inside, Center, Outside}`を受け取り、
指定矩形に対する線の位置を選べる。従来の`outline`は互換性のためInsideとして動作する。
CenterとOutsideでは外側へ広がった部分も描けるclipを渡す必要がある。

## パネルシステム

`PanelTree<N>`はalloc時に拡張する分割パネルツリーです。allocatorなしでは明示的な固定容量です。`split`で既存パネルを
水平または垂直に分け、`layout`でパネルとスプリッターの矩形を呼出側バッファへ出力する。
スプリッターのドラッグ中は`resize`へルートまたは親パネルの矩形とポインター座標を渡す。

```rust
use lagui::{Rect, layout::Axis, panel::{PanelId, PanelLayout, PanelTree, SplitLayout}};

let mut panels = PanelTree::<7>::new(PanelId(1))?;
let main_split = panels.split(PanelId(1), PanelId(2), Axis::Horizontal, 0.25)?;
panels.set_metrics(6.0, 120.0);

let mut panel_layouts = [PanelLayout { id: PanelId(0), bounds: Rect::default() }; 4];
let mut split_layouts = [SplitLayout::default(); 3];
let (panel_count, split_count) = panels.layout(
    Rect::new(0.0, 0.0, 1280.0, 720.0),
    &mut panel_layouts,
    &mut split_layouts,
)?;
// ドラッグ時: panels.resize(main_split, window_bounds, pointer)?;
```

allocatorなしでは`N`がパネルと分割Nodeの合計容量です。allocでは初期reservationです。
`SplitId`は配列slotと独立し、ドッキングでNodeが移動しても同じ分割を指します。
削除されたSplitIdを使うとSplitNotFoundとなり、後から作成された分割を操作しません。
`PanelTabs<N>`のactive状態も安定したPanelIdで管理します。
タブ名の描画と、安定した`WidgetId`によるタブ・スプリッター登録はアプリ側が行う。
Studio demoでは各領域の左上に現在のパネル名を表示し、ヘッダーから全パネルを選ぶ実装例がある。
ヘッダーを別の領域へドラッグすると、ポインターに最も近い左・右・上・下のドック候補を
半透明で表示する。ドロップ時は`PanelTree::dock`が移動元の空いた分割を縮約し、移動先を
新しく二分割する。再構成後のスプリッターも`layout`の結果から動的に登録する。

## 段落と通常の入力欄

`portable`の`text::paragraph::Paragraph`は共有FontSystemで段落をshape/layoutします。
同じtext/styleを設定すると再shapeを省略します。折り返し・双方向文字・fallbackを
同じbufferで扱い、hit_testはDocumentに渡せるUTF-8 byte位置とaffinityを返します。
selection_rectsは混在方向の選択を複数矩形で返します。buffer()は読み取り専用で、
`paragraph::raster::TextArea`を使って既存Glyphon passへ直接描画できます。
Documentを編集する機能は持ちません。解析的なFont/TextRunとは用途で選択し、
どちらの経路でもホストがFontSystem/device/windowを所有します。
