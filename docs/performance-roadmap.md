# 描画性能の改善設計

2026-10-25 作成。SlugVulkanGUI（ken-eizo/SlugVulkanGUI）との比較で見つかった不足を、
効果の大きい順に 7 項目の実装設計としてまとめる。番号順に実装する。
各項目は独立したコミット（必要なら複数）にし、項目ごとに CHANGELOG を更新する。

前提として守ること（CONTRIBUTING.md、Graphy AGENTS.md と共通）:

- painter's order を変えない。バッチ化・カリングで重なり順が変わる実装は不可。
- 可変長データに固定 capacity を設けない。GPU buffer は不足時に拡張し、容量エラーは原子的に返す。
- 同じ情報を二重に持たない。CPU mirror は「GPU に何が載っているか」の唯一の写しとして扱う。
- hover / pan / zoom / selection だけで atlas 再構築・shaping・全量アップロードを起こさない。
- 性能の主張は 6 の計測基盤で測った数値だけで行う。

現状の参照箇所:

| 対象 | ファイル |
|---|---|
| 差分アップロードと Damage | `src/gpu/instances.rs` |
| 角丸 Visual（76 B/instance） | `src/gpu.rs`, `native/visual.wgsl`, `native/corners.wgsl` |
| Windfoil 共通 core（72 B/instance） | `src/windfoil/gpu.rs`, `native/windfoil.wgsl` |
| vector path atlas | `src/windfoil.rs` |
| 文字 | `src/text/renderer.rs`, `src/text/mod.rs` |
| emoji | `src/gpu/emoji.rs`, `native/emoji.wgsl` |
| Graphy 側の合成 | `src/studio/compositor.rs`, `src/studio/application/app_render.rs`, `src/studio/connections.rs` |

---

## 1. GPU 側の 2D 変換（パン・ズームを転送ゼロにする）

### 問題

- インスタンスは論理 pixel のスクリーン座標で encode される（Visual の `shape_bounds`、
  Windfoil の `origin`/`scale`）。uniform は viewport・scale_factor・gamma の 16 B だけ。
- Node Editor / Viewer をパン・ズームすると全インスタンスのバイト列が変わり、差分アップロードが
  全量転送になる。Damage も Full になる。
- Graphy の接続線（`src/studio/connections.rs`）はスクリーン座標の path から毎回 `Atlas` を作り直して
  `sync_atlas` しているため、パンのたびに曲線分解・band 構築・atlas 転送が走る。

### 設計

**変換テーブル** `lagui::gpu::Transforms` を追加する。

```rust,ignore
/// Axis-aligned similarity: screen = local * scale + offset (logical pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform2D { pub offset: [f32; 2], pub scale: f32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TransformId(u32);
impl TransformId { pub const IDENTITY: Self = Self(0); }

pub struct Transforms { /* CPU mirror Vec<Transform2D>, GPU storage buffer, bind group */ }
impl Transforms {
    pub fn new(device: &wgpu::Device) -> Self;
    /// Frame-scoped: ids are valid until the next `begin`.
    pub fn begin(&mut self);
    pub fn push(&mut self, t: Transform2D) -> TransformId;
    pub fn commit(&mut self, queue: &wgpu::Queue);
    /// Ids whose value differs from the previous commit (for damage).
    pub fn changed(&self) -> impl Iterator<Item = TransformId> + '_;
    pub fn bind(&self, pass: &mut wgpu::RenderPass<'_>); // group 1
}
```

- 等方スケール＋平行移動だけにする。`Viewport2D { offset, scale }` からそのまま作れる。回転・非等方は
  UI キャンバスに不要で、角丸半径・stroke 幅・shadow を 1 つの scale で一貫して拡大できる。
- index 0 は常に identity。テーブルは `Vec` で拡張し、GPU 側 storage buffer は不足時 2 倍に再作成する
  （bind group も作り直す）。
- 所有者: 変換は host のカメラ（一時的な操作状態）から毎フレーム作る presentation データ。Lagui は
  frame 内の写しだけを持ち、保存しない。
- 全 Lagui pipeline の layout の group 1 を同じ `Transforms` layout にする。layout は
  `lagui::gpu::Shared::new(device)`（新設。transform layout と後述の共有 atlas layout を持つ）から取り、
  各 Renderer / Pipeline の constructor に渡す。group 1 は pass ごとに一度 `bind` すればよい
  （layout が互換なので `set_pipeline` 後も保持される）。

**インスタンス**

- Visual: `transform: u32` を追加（76 → 80 B）。`shape_bounds` はローカル座標、`clip` は常に
  スクリーン座標（パネルの clip はカメラで動かない）。radius・stroke_width・shadow offset/blur/spread は
  ローカル単位として `scale` を掛ける。
- Windfoil: `transform: u32` を追加。`origin`/`scale` はローカル。
- 文字の baseline snap（`src/text/renderer.rs` の `snap`）はスクリーン座標で行う必要があるため
  vertex shader に移す。instance に `baseline_y: f32`（ローカル）を足し、
  `screen_baseline = round((baseline_y * s + oy) * dpi) / dpi` を求め、glyph の y offset
  （`position[1] * size`）はその後に足す。Windfoil instance は 72 → 80 B。
  snap しない vector shape は `baseline_y = NaN` ではなく、flag bit で区別する
  （`row_range` の上位ビットなど、既存の u32 に余りがあればそこを使う）。
- `VisualList` の各 `Visual` に `transform: TransformId`（default IDENTITY）、`TextSection` と
  `windfoil::Instance` に `transform` を追加する。builder API は
  `list.with_transform(id, |list| { ... })` で範囲指定できるようにする。

**Damage**

- `InstanceBuffer::commit` の `bounds` を `bounds(bytes, &Transforms)` にし、スクリーン座標の範囲を返す。
- `Transforms::changed()` に含まれる id を参照する instance は、旧変換の範囲と新変換の範囲を damage に
  含める（どちらも clip と交差させる）。アップロードは発生しない。旧変換は `Transforms` が前回値を
  保持して提供する。
- viewport / scale_factor の変更は従来どおり Full。

**接続線（Graphy 側）**

- `connections.rs` の path をグラフ座標で作り、カメラ変換を `TransformId` で渡す。
- 曲線近似の精度はズームに依存するため、`scale` を 2 の冪で量子化した LOD を atlas の key にし、
  LOD が変わったときだけ atlas を作り直す。パンでは作り直さない。
- 可視判定（`visible_path`）はパンで変わるため、atlas に入れる範囲は「clip を各方向に 1 画面分広げた
  guard 領域」で決め、カメラが guard を出たときだけ作り直す。

### 検証

- 単体: 同じ VisualList をパンしたとき、2 回目の `prepare` の upload bytes が 0（変換テーブルの
  数十バイトを除く）で、Damage が旧範囲∪新範囲になる。
- GPU readback: identity 変換と、スクリーン座標を手で変換して渡した旧経路の画像が pixel 一致する
  （文字の baseline snap を含む）。scale 0.25 / 1 / 3.7 / 8 で AA 幅が一定。
- Graphy: 1000 ノードのグラフをパンしたときに接続線 atlas の再構築回数が 0。

---

## 2. 文字の CPU カリング

### 問題

`TextRenderer::encode` は run の全 glyph を instance 化し、clip は vertex shader で quad を潰して処理する。
長いリスト・スクロールした文書・パネル外のラベルでも全 glyph の encode・差分比較・転送・頂点処理が走る。

### 設計

- `TextRun` に shape 時点で ink bounds（em 単位、全 glyph bounds の和）を持たせる。
- `TextSection` に `cull: Cull` を追加する。

```rust,ignore
pub enum Cull {
    /// Drop glyphs outside `clip` (default for identity transforms).
    Clip,
    /// Drop glyphs outside `clip` grown by `margin` logical pixels on every side.
    /// Keeps the emitted set stable while a transformed layer pans inside the guard.
    Guard { margin: f32 },
    Off,
}
```

- 判定は 2 段階: section 全体（ink bounds）が clip 外なら丸ごと 0 instance、交差するなら glyph ごとに
  bounds を判定。判定はスクリーン座標で、1 の変換を CPU でも同じ式で適用する（shader の式と同一の
  関数を `windfoil/gpu.rs` に置き、damage 計算と共用する）。
- 個数が変わるため、host が glyph 数から draw range を計算する方式（Graphy の `glyph_ends` /
  `glyph_ranges`）は使えなくなる。`prepare` は section ごとの出力範囲を返す:
  `prepare(...) -> Result<&[Range<u32>], TextError>`（内部の `Vec` を再利用し、warm 時は確保しない）。
  Graphy の `renderer_host.rs` / `compositor.rs` はこの範囲を使うよう変更する。
- 差分アップロードとの関係: カリングで先頭の glyph が消えると後続の index がずれて差分が効かない。
  スクロール中はどのみち位置が全部変わるので問題にならない。静止中は出力が安定するので差分が効く。
  変換レイヤーでは `Guard` を使い、パンのたびに集合が変わらないようにする。
- 同じ helper（`fn visible(bounds, clip, cull) -> bool`）を Visual と windfoil `Renderer` にも適用する。
  ただし Visual は host が既に範囲外を作らないことが多いので、Text を先に入れ、Visual は計測で効果が
  出る場合だけ有効にする。

### 検証

- 1 万 glyph の文書で clip が 1 行分のとき、instance 数が可視行の glyph 数と一致する。
- 端で一部だけ見える glyph が残る（clip 境界の AA を壊さない）。
- painter's order: 複数 section・複数 font が交互に並ぶ場合の draw range が正しい。
- warm 時の追加確保ゼロ（既存の allocator 検査を拡張）。

---

## 3. 描画バッチの統合

### 問題

- フォントごとに `TextRenderer` が別の outline buffer / bind group を持つため、フォントが変わるたびに
  draw が分かれる。接続線の windfoil `Renderer` も別 buffer。
- Visual・Windfoil・emoji が別 pipeline のため、ラベルとパネルが交互に並ぶ UI では
  `compositor.rs` の合成ループで `set_pipeline` と draw が細かく分かれる。

### 3a. 共有 outline atlas（先に実装、低リスク）

- `lagui::gpu::Shared` に、全フォント・全 vector shape の curves / rows を 1 組の storage buffer に置く
  `OutlineAtlas` を持たせる。glyph と shape は `(font_id or shape namespace, id)` から band start で引く。
- append-only。容量不足時は buffer を 2 倍で再作成し、generation を進める。古い generation を参照する
  TextRun / Shape は従来の `AtlasOutOfDate` と同様にエラーで返す（暗黙に壊さない）。
- 不要になった shape（接続線の古い LOD など）は region 単位の解放リストで再利用する。断片化が
  閾値を超えたら compaction（全 shape の再配置＋generation 更新）を scene 境界で行う。
- これで文字（全フォント）と接続線が同じ Windfoil pipeline・同じ bind group・同じ instance buffer になり、
  隣接していれば 1 draw になる。

### 3b. 単一 UI pipeline（計測で採否を決める）

- 全種類を 1 pipeline・1 instance stream で描く。instance は可変長レコードにする（vertex pulling）:
  - vertex buffer は使わず、`records: array<u32>`（storage）と `index: array<u32>`（storage、
    1 instance = 1 要素。上位 4 bit が kind、下位 28 bit がレコードの word offset）にする。
  - kind ごとのレコードは現在の encode をそのまま使う（RoundedRect 80 B、Outline 80 B、Image）。
    union で 1 つの固定サイズにしないので帯域が増えない。
  - `draw(0..6, 0..n)` の instance_index で `index` を引き、kind で分岐して decode する。
- fragment は kind で分岐する。同じ warp に別 kind の quad が混ざると分岐コストが出る。また uber shader の
  レジスタ使用量が最大の kind（Outline）に揃い、単純な矩形の occupancy が下がる可能性がある。
  → 6 の GPU timestamp で「現状の分割 pipeline」と「3b」を同じシーンで比較し、勝つ場合だけ既定にする。
  負ける場合は 3a だけを残し、3b は feature の下に置くか削除する。
- 差分アップロードは `records` と `index` の両方に対して既存の `InstanceBuffer` を使う。
- Graphy の `compositor.rs` は label / emoji / connection の挿入位置を突き合わせる合成ループが不要になり、
  GPU canvas（Preview・blur など）の境界だけで pass を分ける形に単純化される。

### 検証

- 3a: 3 フォント＋接続線が交互に並ぶシーンで draw call 数が「GPU canvas 境界の数＋1」以下になる。
- 3b: 現行と pixel 一致。採否は 6 の bench シーン（混在 UI、長文、1000 ノード）の p50/p99 GPU 時間で判断し、
  結果を `docs/performance.md` に記録する。

---

## 4. Damage を使った部分再描画

### 問題

`Damage` は各 Renderer で計算済みだが、host は毎フレーム画面全体を描き直す。hover 1 個の変化でも
全画面の coverage shader が走る。

### 設計

- swapchain image は present 後に内容が保証されず、数枚が巡回するため、そのままでは前フレームの
  内容を再利用できない。そこで `lagui::gpu::RetainedTarget`（surface と同じサイズ・format の永続 texture）
  を追加する。
  1. 全 Renderer の damage と host 側 GPU canvas の damage を union する。
  2. `Damage::None` なら描画も present もしない（frame を出さない）。
  3. `Region(r)` なら `r` を physical pixel に外側へ丸め、`LoadOp::Load` ＋ scissor で RetainedTarget に描く。
     instance は全部 draw してよい（ラスタライズは scissor で制限される）。
  4. RetainedTarget を swapchain へ全画面 blit する（1 quad、帯域は W×H×4×2。4K でも数十 µs 程度の見込みで、
     6 で実測する）。
  5. `Full`（初回・resize・scale 変更・RetainedTarget 再作成）は全面を描く。
- host 側の damage 契約:
  - GPU canvas（3D preview、画像 preview、color picker など Lagui 外の描画）は内容が変わったフレームで
    自分の bounds を damage として報告する。報告しない canvas は毎フレーム Full とみなす（安全側）。
  - backdrop blur のように下の内容を読む層は、damage がその領域（＋blur 半径）と交差したら、
    その領域全体を damage に加える。これを 1 回の固定点計算で閉じる（blur 同士の連鎖も含める）。
- v1 は union 1 矩形。離れた 2 箇所（左上の hover と右下の時計）で全面に近くなる場合のために、v2 で
  `Vec<Rect>`（固定上限なし、面積が近いものを統合）にし、矩形ごとに scissor を変えて draw を繰り返す。
  v2 は 6 で v1 の damage 面積率を計測してから判断する。
- 3 の後に実装する（pipeline 統合後の方が scissor ごとの再発行が安い）。

### 検証

- GPU readback: 全面描画と部分描画を交互に行ったシーケンスで、毎フレームの最終画像が全面描画と pixel 一致する
  （hover、文字入力、blur の下の変化、resize 直後）。
- hover だけのフレームで fragment 起動数（あるいは GPU 時間）が damage 面積に比例して下がる。

---

## 5. 低遅延のフレーム順序

### 問題

Graphy の `render_frame` は「入力処理（tick / actions）→ surface acquire（待ち）→ 評価 → UI 構築 →
prepare → submit/present」の順。acquire の待ち時間だけ入力が古くなる。Lagui は
`desired_maximum_frame_latency: 1` と present mode の指定以外に、この順序の契約を持たない。

### 設計

`lagui::frame` に host が従う順序と部品を置く（winit 依存は `portable` feature 側）。

1. **dirty 判定**: 入力イベント・アニメーション・外部通知で dirty になったときだけ frame を始める。
   dirty でなければ acquire しない（4 の `Damage::None` より前の安い判定）。
2. **wait/acquire**: 前フレームの submission 完了を待ち（`device.poll` で前回の submission index まで）、
   surface texture を acquire する。待ちはすべてここに集める。
3. **入力 snapshot の確定**: 2 の後に溜まった入力を frame snapshot に確定する（edge は確定済みのものだけ）。
4. **pointer の再サンプル**: drag / slider を宣言する直前に、OS から絶対位置だけを取り直す。
   `platform` に契約を追加する:

   ```rust,ignore
   pub trait PointerProbe {
       /// Current pointer in window logical pixels, if the OS can report it synchronously.
       fn pointer(&self) -> Option<[f32; 2]>;
   }
   ```

   Windows は `GetCursorPos` + `ScreenToClient`、macOS は `mouseLocationOutsideOfEventStream`。
   取れない環境では `None` で最後のイベント位置を使う。button / key の edge は変えず、位置と delta だけ更新する。
5. 構築 → prepare → encode → submit → present。

- present mode は `PresentPolicy { Vsync, LowLatency, Tearing }` にまとめる。
  LowLatency = Mailbox → Immediate → Fifo、Tearing = Immediate → Mailbox → Fifo。実際に選ばれた mode を
  stats に必ず出す。
- Graphy 側は `app_render.rs` の acquire を tick の前に移す。ただし評価（`evaluate_output`）など重い処理は
  acquire 待ちと並行できないので、acquire 前に済ませてよいもの（入力に依存しない評価の publish）と、
  acquire 後でなければならないもの（入力 snapshot・UI 構築）を分けて並べ直す。

### 検証

- 6 の計測で「最終入力 sample から submit まで」の CPU 時間と、acquire 待ちが入力 sample より前にあることを
  trace で確認する。
- PresentMon（Windows）で、変更前後の present から表示までの時間と、入力から present までの時間を比較する。

---

## 6. 計測基盤と Windows 実機検証

### 問題

`docs/performance.md` は UI 登録 CPU の計測だけで、GPU 時間・転送量・draw call 数・present 待ちを
分けて測る仕組みがない。実機検証は macOS だけ。1〜5 と 7 の効果を証明できない。

### 設計

**FrameStats**（Lagui 公開 API）

| 項目 | 内容 |
|---|---|
| `encode_us` | Renderer の encode（instance 化） |
| `diff_upload_us`, `uploaded_bytes`, `write_calls` | `InstanceBuffer::commit` の比較・転送 |
| `instances_by_kind`, `culled` | 種類別 instance 数と 2 で除外した数 |
| `draw_calls`, `pipeline_switches` | 合成の結果 |
| `damage_ratio` | damage 面積 / 画面面積 |
| `gpu_pass_ms` | pass ごとの GPU timestamp 差（サンプリング値） |
| `acquire_wait_us`, `submit_us`, `present_us` | 待ちと driver 呼出し |
| `present_mode` | 実際に選ばれた mode |

- GPU timestamp: `Features::TIMESTAMP_QUERY` があるときだけ。render pass の `timestamp_writes` を使い、
  pass 内 query 機能は要求しない。query set と resolve buffer は frames-in-flight＋1 個のリングにし、
  `map_async` の結果は次以降のフレームで非ブロッキングに読む（計測で stall しない）。
  `interval: u32`（0 は query set を作らない）で間引く。
- draw call 数は `draw_range` が `&mut DrawStats` を受け取るか、3b 後は合成側で数える。

**bench シーン**（`examples/bench_*.rs`、JSON Lines 出力。既存 `examples/performance.rs` の形式に合わせる）

1. 1 万 glyph の長文: 静止、スクロール、パン・ズーム。
2. Studio 相当の 500 コントロール: hover だけ、文字入力。
3. 1000 ノード＋接続線のグラフ: パン、ズーム、ノード drag。
4. 文字・矩形・emoji が交互に並ぶ混在 UI（pipeline 切替の負荷）。
5. 長いリストのスクロール（2 のカリング）。

- offscreen texture への描画で GPU 時間を、window 描画で acquire / present を測る。各ケース warm-up 後に
  p50 / p95 / p99 / max とサンプル数、環境（commit、rustc、OS、CPU/GPU、driver、解像度、DPI、present mode、
  電源）を出す。
- 比較: 同じシーンを SlugVulkanGUI の kitchen_sink 相当で組み、同一機・同一解像度・同一 present mode で
  CPU build / GPU / present を比べる。結果は数値と条件ごと `docs/performance.md` に記録する。

**Windows 実機**

- 開発機（Windows 11）で DX12 と Vulkan の両 backend を `scripts/check.sh gpu` と bench で回す。
  backend 選択は環境変数（`LAGUI_BACKEND=dx12|vulkan`）で切り替え可能にする。
- 入力から表示までは PresentMon（`MsUntilDisplayed`、PC latency 系の列）で測る。カメラによる
  input-to-photon は別途で、ソフトウェア計測で代用しない。

### 進め方の注意

3b の採否と 4 v2 の要否は 6 の数値で決める。6 の GPU timestamp と bench シーン 4 だけは 3b の前に
先行して入れてよい。

---

## 7. Windfoil の band 構造

### 問題

- Windfoil は水平 row だけで curve を分ける。row 内の curve は xmax 降順で並び、pixel の左側
  （`xmax <= -hx`）に来た時点で打ち切るが、row の左端付近の pixel は row 内のほぼ全 curve を評価する。
- 幅の広い shape（接続線、長い stroke、横長の glyph 列を 1 shape にした vector）ほど 1 pixel あたりの
  評価 curve 数が増える。

### 設計: 両方向の積分

- winding は +x 方向の ray でも −x 方向の ray でも求まる（閉曲線なら水平線上の符号付き交差の和は 0 なので、
  左向きは符号を反転するだけ）。box-filter の面積積分も同様に左向きで書ける。
- row ごとに 2 つの curve 列を持つ: 現行の「xmax 降順」（右向き積分、`xmax <= -hx` で打ち切り）と
  「xmin 昇順」（左向き積分、`xmin >= hx` で打ち切り）。curve 本体は複製せず index 列を 2 本にする
  （`rows` に start2 を追加して `ROW_STRIDE` 5 → 6、`curves` は index 経由に変える）。
- row に分割点 `x_split`（row 内 curve の x 中心の中央値）を持たせ、pixel の x が `x_split` より左なら
  左向き、右なら右向きを使う。平均の評価 curve 数がおおむね半分になる見込み。
- index 経由の参照で 1 回の間接読み込みが増えるため、curve 数が少ない row（`SORT_MIN` 以下）は
  現行どおり片方向のまま直接参照する。
- 縮小時の `profile_face` 経路は変更しない。

### 補足: 動的な曲線

ノード drag 中の接続線のように毎フレーム形が変わる曲線は、atlas に入れ直すより、SlugVulkanGUI の
`cubicBezier` のように画面空間で解析的な stroke segment（1 segment = 1 instance、分割はズームに応じた
誤差上限で決める）として描く方が速い可能性がある。drag 中の接続線だけをこの primitive に切り替え、
drag 終了時に atlas へ戻す。6 のシーン 3（ノード drag）で比較してから決める。

### 検証

- 既存の shader 回帰（独立した参照形状と GPU readback）が全て一致。穴・接線・退化曲線・極小サイズを含む。
- 片方向と両方向で coverage の差が許容誤差内（現行の参照テストと同じ閾値）。
- bench シーン 1 と 3 の GPU 時間を変更前後で比較し、`docs/performance.md` に記録。

---

## 実装順と依存

| 順 | 項目 | 依存 | 主な API 変更 |
|---|---|---|---|
| 1 | GPU 変換 | なし | `Shared`、`Transforms`、`TransformId`、instance に transform |
| 2 | 文字カリング | 1（変換後の座標で判定） | `TextSection::cull`、`prepare` が範囲を返す |
| 3a | 共有 outline atlas | 1 | `OutlineAtlas`、Renderer が atlas を共有 |
| 3b | 単一 UI pipeline | 3a、6（採否） | 内部。合成 API が単純化 |
| 4 | 部分再描画 | 1（変換 damage）、3 | `RetainedTarget`、host の damage 契約 |
| 5 | フレーム順序 | なし | `frame` module、`PointerProbe`、`PresentPolicy` |
| 6 | 計測 | なし（3b より前に一部先行可） | `FrameStats`、bench examples |
| 7 | band 構造 | 6（効果確認） | 内部（atlas format と shader） |

Lagui の変更は Lagui リポジトリでコミット・push してから、Graphy でサブモジュール参照の更新と
利用側の変更（`compositor.rs`、`renderer_host.rs`、`app_render.rs`、`connections.rs`）を行う。
