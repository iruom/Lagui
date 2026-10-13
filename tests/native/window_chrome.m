// Run on AppKit's main thread with scripts/check_window_chrome.sh.
#import <AppKit/AppKit.h>
#import <math.h>

extern void bg_install_traffic_light_layout(void *view, double left, double header_height);

@interface BGFlippedTestView : NSView
@end
@implementation BGFlippedTestView
- (BOOL)isFlipped { return YES; }
@end

// Model the final internal frame write of an AppKit layout transaction, which
// does not emit another public frame notification.
@interface BGTransactionButton : NSButton
@property NSRect transactionFrame;
@end
@implementation BGTransactionButton
- (NSRect)frame { return self.transactionFrame; }
- (void)setFrame:(NSRect)frame { self.transactionFrame = frame; }
@end
@interface BGTransactionWindow : NSWindow
@property (strong) NSButton *transactionZoom;
@end
@implementation BGTransactionWindow
- (NSButton *)standardWindowButton:(NSWindowButton)kind {
    if (kind == NSWindowZoomButton && self.transactionZoom) return self.transactionZoom;
    return [super standardWindowButton:kind];
}
@end

static void check(NSView *content, const char *operation) {
    NSWindowButton kinds[] = { NSWindowCloseButton, NSWindowMiniaturizeButton, NSWindowZoomButton };
    CGFloat x = NSMinX(content.bounds) + 14.0;
    CGFloat y = content.isFlipped ? NSMinY(content.bounds) + 21.0 : NSMaxY(content.bounds) - 21.0;
    for (NSUInteger i = 0; i < 3; i++) {
        NSButton *button = [content.window standardWindowButton:kinds[i]];
        NSRect frame = [content convertRect:button.frame fromView:button.superview];
        if (fabs(NSMinX(frame) - x) > 0.01 || fabs(NSMidY(frame) - y) > 0.01) {
            fprintf(stderr, "%s (%s): button %lu at %s; expected left %.2f, center %.2f\n",
                operation, content.isFlipped ? "flipped" : "unflipped", (unsigned long)i,
                NSStringFromRect(frame).UTF8String, (double)x, (double)y);
            exit(1);
        }
        x += frame.size.width + 8.0;
    }
}

static void exercise(BOOL flipped) {
    BGTransactionWindow *window = [[BGTransactionWindow alloc] initWithContentRect:NSMakeRect(100, 100, 640, 480)
        styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable |
            NSWindowStyleMaskResizable | NSWindowStyleMaskFullSizeContentView
        backing:NSBackingStoreBuffered defer:NO];
    window.releasedWhenClosed = NO;
    window.titlebarAppearsTransparent = YES;
    window.titleVisibility = NSWindowTitleHidden;
    NSView *content = flipped ? [[BGFlippedTestView alloc] initWithFrame:window.contentView.bounds]
                             : [[NSView alloc] initWithFrame:window.contentView.bounds];
    window.contentView = content;
    bg_install_traffic_light_layout((__bridge void *)content, 14, 42);
    check(content, "install");
    // Repeat installation must neither accumulate offsets nor duplicate owners.
    bg_install_traffic_light_layout((__bridge void *)content, 14, 42);
    check(content, "repeat install");

    NSWindowButton kinds[] = { NSWindowCloseButton, NSWindowMiniaturizeButton, NSWindowZoomButton };
    for (NSUInteger i = 0; i < 3; i++) {
        NSButton *button = [window standardWindowButton:kinds[i]];
        NSRect frame = button.frame;
        frame.origin = NSMakePoint(0, 0);
        button.frame = frame;
        // No host resize event and no explicit call to the layout helper.
        check(content, "AppKit button frame reset without resize");
    }
    NSButton *nativeZoom = [window standardWindowButton:NSWindowZoomButton];
    BGTransactionButton *transactionZoom = [[BGTransactionButton alloc] init];
    transactionZoom.transactionFrame = nativeZoom.frame;
    [nativeZoom.superview addSubview:transactionZoom];
    window.transactionZoom = transactionZoom;
    bg_install_traffic_light_layout((__bridge void *)content, 14, 42);
    // AppKit can finish a transaction after delivering its frame notification.
    NSButton *zoom = [window standardWindowButton:NSWindowZoomButton];
    [NSNotificationCenter.defaultCenter postNotificationName:NSViewFrameDidChangeNotification object:zoom];
    zoom.postsFrameChangedNotifications = NO;
    zoom.postsBoundsChangedNotifications = NO;
    NSRect reset = zoom.frame;
    reset.origin = NSMakePoint(0, 0);
    zoom.frame = reset;
    zoom.postsFrameChangedNotifications = YES;
    zoom.postsBoundsChangedNotifications = YES;
    __block BOOL drained = NO;
    dispatch_async(dispatch_get_main_queue(), ^{
        check(content, "AppKit transaction completion after notification");
        drained = YES;
    });
    while (!drained) {
        [NSRunLoop.currentRunLoop runMode:NSDefaultRunLoopMode beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.1]];
    }
    window.transactionZoom = nil;
    [transactionZoom removeFromSuperview];
    bg_install_traffic_light_layout((__bridge void *)content, 14, 42);
    NSView *container = [window standardWindowButton:NSWindowCloseButton].superview;
    NSPoint origin = container.frame.origin;
    [container setFrameOrigin:NSMakePoint(origin.x + 3, origin.y + 5)];
    check(content, "titlebar container moved");
    [container setFrameOrigin:origin];
    check(content, "titlebar container restored");

    for (NSUInteger i = 0; i < 6; i++) {
        [window setContentSize:NSMakeSize(640 + i * 37, 480 + i * 19)];
        check(content, "window resized");
    }
    NSView *ancestor = content.superview;
    NSRect previousBounds = ancestor.bounds;
    [ancestor setBoundsOrigin:NSMakePoint(previousBounds.origin.x + 2, previousBounds.origin.y + 4)];
    check(content, "ancestor bounds changed");
    ancestor.bounds = previousBounds;
    check(content, "ancestor bounds restored");
    [window close];
}

int main(void) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        exercise(NO);
        exercise(YES);
        puts("window chrome: resets without resize, idempotence, container movement, resize and flipped coordinates passed");
    }
    return 0;
}
