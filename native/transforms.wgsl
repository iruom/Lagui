struct TransformEntry { placement:vec4f, clip:vec4f }
@group(1) @binding(0) var<storage,read> transforms:array<TransformEntry>;
fn transformed_clip(clip:vec4f,entry:TransformEntry)->vec4f {
    var lo=clip.xy;var hi=clip.xy+clip.zw;
    if entry.placement.w!=0.0 {
        let panel_lo=(entry.clip.xy-entry.placement.xy)/entry.placement.z;
        let panel_hi=(entry.clip.xy+entry.clip.zw-entry.placement.xy)/entry.placement.z;
        lo=max(lo,panel_lo);hi=max(min(hi,panel_hi),lo);
        lo=lo*entry.placement.z+entry.placement.xy;
        hi=hi*entry.placement.z+entry.placement.xy;
    }
    lo=max(lo,entry.clip.xy);hi=max(min(hi,entry.clip.xy+entry.clip.zw),lo);
    return vec4f(lo,hi-lo);
}
