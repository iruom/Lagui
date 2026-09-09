#import <AppKit/AppKit.h>
#import <objc/runtime.h>

static const void *BGTrafficLightOriginalY = &BGTrafficLightOriginalY;
static const void *BGTitlebarDragViewKey = &BGTitlebarDragViewKey;

@interface BGTitlebarDragView : NSView
@end

@implementation BGTitlebarDragView
- (BOOL)isOpaque { return NO; }
- (BOOL)mouseDownCanMoveWindow { return YES; }
- (void)mouseDown:(NSEvent *)event {
    if (event.clickCount == 2) {
        // Keep double-click recognition in AppKit and invoke NSWindow's
        // standard animated zoom/restore action.
        [self.window performZoom:self];
    } else {
        [self.window performWindowDragWithEvent:event];
    }
}
- (NSView *)hitTest:(NSPoint)point {
    if (![self mouse:point inRect:self.bounds]) return nil;
    CGFloat x = point.x;
    if (x < 78.0 || (x >= 296.0 && x < 470.0) || x >= self.bounds.size.width - 110.0) {
        return nil;
    }
    return self;
}
@end

void bg_install_titlebar_drag_region(void *view_pointer, double height) {
    NSView *content = (__bridge NSView *)view_pointer;
    NSWindow *window = content.window;
    if (!window) return;
    // Movement is handled only by BGTitlebarDragView. Enabling background
    // movement here would make the canvas and every panel draggable too.
    [window setMovableByWindowBackground:NO];

    BGTitlebarDragView *dragView = objc_getAssociatedObject(content, BGTitlebarDragViewKey);
    CGFloat y = content.isFlipped ? 0.0 : content.bounds.size.height - height;
    if (!dragView) {
        dragView = [[BGTitlebarDragView alloc] initWithFrame:NSMakeRect(0.0, y, content.bounds.size.width, height)];
        dragView.autoresizingMask = NSViewWidthSizable | (content.isFlipped ? NSViewMaxYMargin : NSViewMinYMargin);
        [content addSubview:dragView positioned:NSWindowAbove relativeTo:nil];
        objc_setAssociatedObject(content, BGTitlebarDragViewKey, dragView,
            OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    } else {
        dragView.frame = NSMakeRect(0.0, y, content.bounds.size.width, height);
    }
}

void bg_position_traffic_lights(void *view_pointer, double left, double down) {
    NSView *content = (__bridge NSView *)view_pointer;
    NSWindow *window = content.window;
    if (!window) return;

    NSButton *buttons[] = {
        [window standardWindowButton:NSWindowCloseButton],
        [window standardWindowButton:NSWindowMiniaturizeButton],
        [window standardWindowButton:NSWindowZoomButton],
    };
    NSView *container = buttons[0].superview;
    if (!container) return;

    CGFloat x = left;
    for (NSUInteger index = 0; index < 3; index++) {
        NSButton *button = buttons[index];
        if (!button || button.superview != container) continue;
        NSRect frame = button.frame;
        NSNumber *storedY = objc_getAssociatedObject(button, BGTrafficLightOriginalY);
        if (!storedY) {
            storedY = @(frame.origin.y);
            objc_setAssociatedObject(button, BGTrafficLightOriginalY, storedY,
                OBJC_ASSOCIATION_RETAIN_NONATOMIC);
        }
        frame.origin.x = x;
        // AppKit titlebar containers normally use a bottom-left origin, so a
        // lower visual position subtracts from Y. Handle flipped containers too.
        frame.origin.y = storedY.doubleValue + (container.isFlipped ? down : -down);
        button.frame = frame;
        x += frame.size.width + 8.0;
    }
}
