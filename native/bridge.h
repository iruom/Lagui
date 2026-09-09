#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>
typedef struct { float x, y, width, height; } BGRect;
typedef struct { BGRect bounds; float color[4]; } BGQuad;
typedef void (*BGInput)(void *, uint32_t, float, float);
typedef size_t (*BGPaint)(void *, BGQuad *, size_t, float, float);
int bg_run(void *, BGInput, BGPaint, const char *, bool);
_Static_assert(sizeof(BGQuad) == 32, "Rust/Metal quad layout mismatch");
