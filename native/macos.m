#import <AppKit/AppKit.h>
#import <Metal/Metal.h>
#import <QuartzCore/CAMetalLayer.h>
#include "bridge.h"

enum { BGCapacity = 96, BGSampleCount = 2048 };
typedef struct { double build, drawable, inputCommit; } BGSample;
static int compareDouble(const void *a, const void *b) {
    double x = *(const double *)a, y = *(const double *)b;
    return (x > y) - (x < y);
}
static void syntheticMouse(NSView *view, NSEventType type, CGFloat x, CGFloat y) {
    NSEvent *event = [NSEvent mouseEventWithType:type
        location:[view convertPoint:NSMakePoint(x,y) toView:nil] modifierFlags:0
        timestamp:CACurrentMediaTime() windowNumber:view.window.windowNumber context:nil
        eventNumber:0x424747 clickCount:1 pressure:type == NSEventTypeLeftMouseDown ? 1 : 0];
    [NSApp sendEvent:event];
}

@interface BGView : NSView
@property void *context;
@property BGInput input;
@property BGPaint paint;
@property id<MTLDevice> device;
@property id<MTLCommandQueue> queue;
@property id<MTLRenderPipelineState> pipeline;
@property CAMetalLayer *metalLayer;
@property BOOL inFlight;
@property BOOL pending;
@property double inputReceived;
@property NSUInteger submitted;
@property NSUInteger failures;
@property NSUInteger sampleCount;
@property NSUInteger sampleCursor;
@property NSTrackingArea *tracking;
@property BOOL smokeMode;
- (void)render;
- (void)report;
- (BOOL)smoke;
@end

@implementation BGView {
    BGQuad _quads[BGCapacity];
    BGSample _samples[BGSampleCount];
}
- (BOOL)isFlipped { return YES; }
- (BOOL)acceptsFirstResponder { return YES; }
- (BOOL)acceptsFirstMouse:(NSEvent *)event { (void)event; return YES; }
- (BOOL)wantsUpdateLayer { return YES; }
- (void)updateLayer { [self render]; }
- (void)updateTrackingAreas {
    [super updateTrackingAreas];
    if (self.tracking) [self removeTrackingArea:self.tracking];
    self.tracking = [[NSTrackingArea alloc] initWithRect:NSZeroRect
        options:NSTrackingMouseMoved | NSTrackingMouseEnteredAndExited | NSTrackingActiveInKeyWindow | NSTrackingInVisibleRect
        owner:self userInfo:nil];
    [self addTrackingArea:self.tracking];
}
- (void)setFrameSize:(NSSize)size {
    [super setFrameSize:size];
    [self render];
}
- (void)viewDidChangeBackingProperties {
    [super viewDidChangeBackingProperties];
    [self render];
}
- (void)send:(uint32_t)kind event:(NSEvent *)event {
    if (self.smokeMode && event.eventNumber != 0x424747) return;
    self.inputReceived = CACurrentMediaTime();
    NSPoint p = [self convertPoint:event.locationInWindow fromView:nil];
    self.input(self.context, kind, p.x, p.y);
    [self render];
}
- (void)mouseMoved:(NSEvent *)e { [self send:0 event:e]; }
- (void)mouseDragged:(NSEvent *)e { [self send:0 event:e]; }
- (void)mouseDown:(NSEvent *)e { [self.window makeFirstResponder:self]; [self send:1 event:e]; }
- (void)mouseUp:(NSEvent *)e { [self send:2 event:e]; }
- (void)mouseExited:(NSEvent *)e {
    // Drag capture must survive leaving the view.
    if (!(NSEvent.pressedMouseButtons & 1)) [self send:3 event:e];
}
- (void)keyDown:(NSEvent *)e {
    if (self.smokeMode) return;
    if ([e.charactersIgnoringModifiers.lowercaseString isEqualToString:@"p"]) { [self report]; return; }
    if (e.keyCode == 49) [self send:4 event:e];
    else if (e.keyCode == 123) [self send:5 event:e];
    else if (e.keyCode == 124) [self send:6 event:e];
    else [super keyDown:e];
}
- (id<MTLCommandBuffer>)encode:(id<MTLTexture>)texture count:(size_t)count {
    id<MTLCommandBuffer> command = [self.queue commandBuffer];
    if (!command) return nil;
    MTLRenderPassDescriptor *pass = [MTLRenderPassDescriptor renderPassDescriptor];
    pass.colorAttachments[0].texture = texture;
    pass.colorAttachments[0].loadAction = MTLLoadActionClear;
    pass.colorAttachments[0].storeAction = MTLStoreActionStore;
    pass.colorAttachments[0].clearColor = MTLClearColorMake(0.014, 0.019, 0.025, 1);
    id<MTLRenderCommandEncoder> encoder = [command renderCommandEncoderWithDescriptor:pass];
    if (!encoder) return nil;
    [encoder setRenderPipelineState:self.pipeline];
    float viewport[] = {(float)self.bounds.size.width, (float)self.bounds.size.height};
    [encoder setVertexBytes:_quads length:count * sizeof(BGQuad) atIndex:0];
    [encoder setVertexBytes:viewport length:sizeof(viewport) atIndex:1];
    [encoder drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:6 instanceCount:count];
    [encoder endEncoding];
    return command;
}
- (void)render {
    if (!self.pipeline || !self.paint || !self.window.visible || self.bounds.size.width <= 0 || self.bounds.size.height <= 0) return;
    if (self.inFlight) { self.pending = YES; return; }
    self.pending = NO;
    double start = CACurrentMediaTime();
    size_t count = self.paint(self.context, _quads, BGCapacity, self.bounds.size.width, self.bounds.size.height);
    double built = CACurrentMediaTime();
    if (!count || count > BGCapacity) { self.failures++; return; }
    CGFloat scale = self.window.backingScaleFactor;
    self.metalLayer.contentsScale = scale;
    self.metalLayer.drawableSize = CGSizeMake(self.bounds.size.width * scale, self.bounds.size.height * scale);
    double acquire = CACurrentMediaTime();
    id<CAMetalDrawable> drawable = [self.metalLayer nextDrawable];
    double acquired = CACurrentMediaTime();
    if (!drawable) {
        // No busy-spin when the window is occluded or the drawable pool is unavailable.
        __weak BGView *weak = self;
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 16 * NSEC_PER_MSEC), dispatch_get_main_queue(), ^{
            if (weak.window.visible) [weak render];
        });
        return;
    }
    id<MTLCommandBuffer> command = [self encode:drawable.texture count:count];
    if (!command) { self.failures++; return; }
    self.inFlight = YES;
    __weak BGView *weak = self;
    [command addCompletedHandler:^(id<MTLCommandBuffer> completed) {
        BOOL failed = completed.status == MTLCommandBufferStatusError;
        if (failed) fprintf(stderr, "Metal command failed: %s\n", completed.error.description.UTF8String);
        dispatch_async(dispatch_get_main_queue(), ^{
            BGView *view = weak;
            if (!view) return;
            if (failed) view.failures++;
            view.inFlight = NO;
            if (view.pending) [view render];
        });
    }];
    [command presentDrawable:drawable];
    [command commit];
    double committed = CACurrentMediaTime();
    self.submitted++;
    if (self.inputReceived > 0) {
        _samples[self.sampleCursor] = (BGSample){(built-start)*1e6, (acquired-acquire)*1e6, (committed-self.inputReceived)*1e6};
        self.sampleCursor = (self.sampleCursor + 1) % BGSampleCount;
        self.sampleCount = MIN(self.sampleCount + 1, BGSampleCount);
        self.inputReceived = 0;
    }
}
- (void)report {
    fprintf(stdout, "frames=%lu errors=%lu input_samples=%lu (latest event received -> commit; not display latency)\n",
        (unsigned long)self.submitted, (unsigned long)self.failures, (unsigned long)self.sampleCount);
    if (!self.sampleCount) return;
    const char *names[] = {"Rust paint", "nextDrawable wait", "input receive -> commit"};
    for (int metric = 0; metric < 3; metric++) {
        double values[BGSampleCount];
        for (NSUInteger i = 0; i < self.sampleCount; i++) {
            BGSample s = _samples[i];
            values[i] = metric == 0 ? s.build : metric == 1 ? s.drawable : s.inputCommit;
        }
        qsort(values, self.sampleCount, sizeof(double), compareDouble);
        NSUInteger n = self.sampleCount - 1;
        fprintf(stdout, "%s: p50 %.3f us | p95 %.3f us | p99 %.3f us | max %.3f us\n", names[metric],
            values[(NSUInteger)(n*.5)], values[(NSUInteger)(n*.95)], values[(NSUInteger)(n*.99)], values[n]);
    }
    fflush(stdout);
}
- (BOOL)smoke {
    // This is synthetic input, separate from user input-to-display measurements.
    syntheticMouse(self, NSEventTypeLeftMouseDown, 80, 175);
    syntheticMouse(self, NSEventTypeLeftMouseUp, 80, 175);
    syntheticMouse(self, NSEventTypeLeftMouseDown, 260, 275);
    syntheticMouse(self, NSEventTypeLeftMouseUp, 260, 275);
    size_t count = self.paint(self.context, _quads, BGCapacity, self.bounds.size.width, self.bounds.size.height);
    if (!count || count > BGCapacity) return NO;
    NSUInteger width = (NSUInteger)self.bounds.size.width, height = (NSUInteger)self.bounds.size.height;
    MTLTextureDescriptor *desc = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatBGRA8Unorm_sRGB
        width:width height:height mipmapped:NO];
    desc.usage = MTLTextureUsageRenderTarget;
    desc.storageMode = MTLStorageModeShared;
    id<MTLTexture> texture = [self.device newTextureWithDescriptor:desc];
    if (!texture) return NO;
    id<MTLCommandBuffer> command = [self encode:texture count:count];
    if (!command) return NO;
    [command commit];
    [command waitUntilCompleted]; // Only the smoke/readback path blocks on the GPU.
    if (command.status != MTLCommandBufferStatusCompleted) return NO;
    NSMutableData *bytes = [NSMutableData dataWithLength:width*height*4];
    [texture getBytes:bytes.mutableBytes bytesPerRow:width*4 fromRegion:MTLRegionMake2D(0,0,width,height) mipmapLevel:0];
    uint8_t *p = bytes.mutableBytes;
    // Verify an accent pixel against the Rust state after button activation.
    NSUInteger offset = (42*width + 45)*4;
    if (!(p[offset] > 240 && p[offset+1] > 125 && p[offset+1] < 145 && p[offset+2] > 195 && p[offset+2] < 210 && p[offset+3] == 255)) {
        fprintf(stderr, "smoke accent mismatch: BGRA=%u,%u,%u,%u; hit=%s\n", p[offset],p[offset+1],p[offset+2],p[offset+3],
            NSStringFromClass([self hitTest:NSMakePoint(80,175)].class).UTF8String);
        return NO;
    }
    for (NSUInteger i = 0; i < width*height; i++) { uint8_t b=p[i*4]; p[i*4]=p[i*4+2]; p[i*4+2]=b; }
    NSBitmapImageRep *rep = [[NSBitmapImageRep alloc] initWithBitmapDataPlanes:NULL pixelsWide:width pixelsHigh:height
        bitsPerSample:8 samplesPerPixel:4 hasAlpha:YES isPlanar:NO colorSpaceName:NSDeviceRGBColorSpace bytesPerRow:width*4 bitsPerPixel:32];
    memcpy(rep.bitmapData, p, bytes.length);
    [[NSFileManager defaultManager] createDirectoryAtPath:@"build" withIntermediateDirectories:YES attributes:nil error:NULL];
    BOOL written = [[rep representationUsingType:NSBitmapImageFileTypePNG properties:@{}] writeToFile:@"build/native-smoke.png" atomically:YES];
    fprintf(stdout, "smoke: %zu quads, verified Metal pixel, GPU %.3f us, capture=build/native-smoke.png (GPU-only capture; AppKit labels excluded)\n", count, (command.GPUEndTime-command.GPUStartTime)*1e6);
    return written;
}
@end

@interface BGDelegate : NSObject <NSApplicationDelegate, NSWindowDelegate>
@property NSWindow *window;
@property BGView *view;
@end
@implementation BGDelegate
- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication *)app { (void)app; return YES; }
- (void)windowDidResignKey:(NSNotification *)note {
    (void)note;
    self.view.input(self.view.context, 3, 0, 0);
    [self.view render];
}
- (void)windowWillClose:(NSNotification *)note { (void)note; [self.view report]; }
@end

@interface BGLabel : NSTextField
@end
@implementation BGLabel
- (NSView *)hitTest:(NSPoint)point { (void)point; return nil; }
@end

static void label(BGView *view, NSString *text, NSRect frame, CGFloat size, NSColor *color) {
    BGLabel *label = [BGLabel labelWithString:text];
    label.frame = frame;
    label.font = [NSFont systemFontOfSize:size weight:NSFontWeightMedium];
    label.textColor = color;
    label.selectable = NO;
    [view addSubview:label];
}

int bg_run(void *context, BGInput input, BGPaint paint, const char *shader, bool smoke) {
    @autoreleasepool {
        NSApplication *app = [NSApplication sharedApplication];
        [app setActivationPolicy:NSApplicationActivationPolicyRegular];
        BGDelegate *delegate = [BGDelegate new];
        app.delegate = delegate;
        BGView *view = [[BGView alloc] initWithFrame:NSMakeRect(0,0,960,600)];
        view.context = context; view.input = input; view.paint = paint;
        view.smokeMode = smoke;
        view.device = MTLCreateSystemDefaultDevice();
        if (!view.device) { fprintf(stderr, "No Metal device available\n"); return 1; }
        view.queue = [view.device newCommandQueue];
        NSError *error = nil;
        id<MTLLibrary> library = [view.device newLibraryWithSource:[NSString stringWithUTF8String:shader] options:nil error:&error];
        if (!library) { fprintf(stderr, "Shader compilation failed: %s\n", error.description.UTF8String); return 2; }
        MTLRenderPipelineDescriptor *pipeline = [MTLRenderPipelineDescriptor new];
        pipeline.vertexFunction = [library newFunctionWithName:@"quad_vertex"];
        pipeline.fragmentFunction = [library newFunctionWithName:@"quad_fragment"];
        pipeline.colorAttachments[0].pixelFormat = MTLPixelFormatBGRA8Unorm_sRGB;
        pipeline.colorAttachments[0].blendingEnabled = YES;
        pipeline.colorAttachments[0].sourceRGBBlendFactor = MTLBlendFactorOne;
        pipeline.colorAttachments[0].destinationRGBBlendFactor = MTLBlendFactorOneMinusSourceAlpha;
        pipeline.colorAttachments[0].sourceAlphaBlendFactor = MTLBlendFactorOne;
        pipeline.colorAttachments[0].destinationAlphaBlendFactor = MTLBlendFactorOneMinusSourceAlpha;
        view.pipeline = [view.device newRenderPipelineStateWithDescriptor:pipeline error:&error];
        if (!view.pipeline || !view.queue) { fprintf(stderr, "Metal setup failed: %s\n", error.description.UTF8String); return 3; }
        view.metalLayer = [CAMetalLayer layer];
        view.metalLayer.device = view.device;
        view.metalLayer.pixelFormat = MTLPixelFormatBGRA8Unorm_sRGB;
        view.metalLayer.maximumDrawableCount = 2;
        const char *sync = getenv("BEST_GUI_VSYNC");
        view.metalLayer.displaySyncEnabled = sync && strcmp(sync, "1") == 0;
        fprintf(stdout, "Metal: %s; display sync=%s; at most one GPU command in flight\n", view.device.name.UTF8String,
            view.metalLayer.displaySyncEnabled ? "on" : "off");
        view.layer = view.metalLayer;
        view.wantsLayer = YES;
        NSWindow *window = [[NSWindow alloc] initWithContentRect:view.frame
            styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable
            backing:NSBackingStoreBuffered defer:NO];
        window.title = @"Best GUI · Rust + Metal   |   Space: accent   ← →: slider   P: metrics   ⌘Q: quit";
        window.releasedWhenClosed = NO;
        window.contentView = view;
        window.acceptsMouseMovedEvents = YES;
        window.delegate = delegate;
        delegate.window = window; delegate.view = view;
        // Temporary system-rendered labels; the custom text backend is not implemented yet.
        label(view, @"BEST GUI", NSMakeRect(40,64,260,35), 28, NSColor.whiteColor);
        label(view, @"A native beginning.", NSMakeRect(40,108,260,24), 15, NSColor.secondaryLabelColor);
        label(view, @"Change accent", NSMakeRect(60,164,140,24), 15, NSColor.whiteColor);
        label(view, @"INTENSITY", NSMakeRect(40,226,260,24), 12, NSColor.secondaryLabelColor);
        label(view, @"Drag the slider. Move the pointer.\nSpace changes the accent.\nArrow keys adjust the intensity.\nP prints timing samples.", NSMakeRect(40,332,275,100), 13, NSColor.secondaryLabelColor);
        label(view, @"Direct to Metal.", NSMakeRect(380,64,380,40), 30, NSColor.whiteColor);
        label(view, @"Event-driven rendering. Rust-owned state.", NSMakeRect(380,110,480,24), 14, NSColor.secondaryLabelColor);
        view.appearance = [NSAppearance appearanceNamed:NSAppearanceNameDarkAqua];
        NSMenu *menu = [NSMenu new];
        NSMenuItem *root = [NSMenuItem new]; [menu addItem:root];
        NSMenu *applicationMenu = [NSMenu new];
        [applicationMenu addItemWithTitle:@"Quit Best GUI" action:@selector(terminate:) keyEquivalent:@"q"];
        root.submenu = applicationMenu; app.mainMenu = menu;
        [window center]; [window makeKeyAndOrderFront:nil]; [window makeFirstResponder:view];
        [app activateIgnoringOtherApps:YES];
        [view render];
        __block int result = 0;
        if (smoke) dispatch_after(dispatch_time(DISPATCH_TIME_NOW, NSEC_PER_SEC), dispatch_get_main_queue(), ^{
            result = [view smoke] && view.submitted > 0 && view.failures == 0 ? 0 : 4;
            [view report];
            [app stop:nil];
        });
        [app run];
        // Rust's context remains valid until all callbacks have stopped.
        view.input = NULL; view.paint = NULL;
        window.delegate = nil; app.delegate = nil;
        [window orderOut:nil];
        return result;
    }
}
