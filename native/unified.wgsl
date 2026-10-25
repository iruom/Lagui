
@group(3) @binding(0) var<storage,read> records:array<u32>;
@group(3) @binding(1) var<storage,read> indices:array<u32>;
struct UnifiedVertex {
 @builtin(position) position:vec4f,
 @location(0) logical:vec2f,
 @location(1) local:vec2f,
 @location(2) @interpolate(flat) extent:vec2f,
 @location(3) @interpolate(flat) color:vec4f,
 @location(4) @interpolate(flat) params:vec3f,
 @location(5) @interpolate(flat) inner_color:vec4f,
 @location(6) @interpolate(flat) shape_bounds:vec4f,
 @location(7) @interpolate(flat) shadow_color:vec4f,
 @location(8) @interpolate(flat) shadow:vec4f,
 @location(9) rc:vec2f,
 @location(10) @interpolate(flat) row_range:vec2u,
 @location(11) @interpolate(flat) band_size:vec2f,
 @location(12) @interpolate(flat) bbox:vec4f,
 @location(13) uv:vec2f,
 @location(14) @interpolate(flat) layer:u32,
 @location(15) @interpolate(flat) kind:u32,
}
fn f4(o:u32)->vec4f {return bitcast<vec4f>(vec4u(records[o],records[o+1u],records[o+2u],records[o+3u]));}
fn h4(o:u32)->vec4f {return vec4f(unpack2x16float(records[o]),unpack2x16float(records[o+1u]));}
@vertex fn vertex_main(@builtin(vertex_index) vertex:u32,@builtin(instance_index) instance:u32)->UnifiedVertex {
 let index=indices[instance];let o=index&0x0fffffffu;let kind=index>>28u;
 var out:UnifiedVertex;out.kind=kind;
 switch kind {
 case 0u: {
 let v=visual_vertex(vertex,f4(o),f4(o+4u),h4(o+8u),h4(o+10u),h4(o+12u),h4(o+14u),f4(o+16u).xyz,records[o+19u]);
 out.position=v.position;out.logical=v.logical;out.local=v.local;out.extent=v.extent;out.color=v.color;out.params=v.params;out.inner_color=v.inner_color;out.shape_bounds=v.shape_bounds;out.shadow_color=v.shadow_color;out.shadow=v.shadow;
 }
 case 1u: {
 let v=outline_vertex(vertex,f4(o),f4(o+4u),f4(o+8u),vec2u(records[o+12u],records[o+13u]),f4(o+14u).xy,h4(o+16u),records[o+18u],bitcast<f32>(records[o+19u]));
 out.position=v.position;out.rc=v.rc;out.color=v.color;out.row_range=v.row_range;out.band_size=v.band_size;out.bbox=v.bbox;
 }
 default: {let v=emoji_vertex(vertex,f4(o),f4(o+4u),records[o+8u]);out.position=v.position;out.uv=v.uv;out.layer=v.layer;}
 }
 return out;
}
@fragment fn fragment_main(v:UnifiedVertex)->@location(0) vec4f {
 switch v.kind {
 case 0u: {return visual_fragment(VisualVertex(v.position,v.logical,v.local,v.extent,v.color,v.params,v.inner_color,v.shape_bounds,v.shadow_color,v.shadow));}
 case 1u: {return outline_fragment(OutlineVertex(v.position,v.rc,v.color,v.row_range,v.band_size,v.bbox));}
 default: {let c=emoji_fragment(EmojiVertex(v.position,v.uv,v.layer));return vec4f(c.rgb*c.a,c.a);}
 }
}
