#import <AppKit/AppKit.h>
#import <objc/runtime.h>
int bg_window_in_live_resize(void *view_pointer) {
    NSView *view = (__bridge NSView *)view_pointer;
    return view.window.inLiveResize ? 1 : 0;
}

int bg_probe_pointer(void *view_pointer, double *x, double *y) {
    NSView *view = (__bridge NSView *)view_pointer;
    if (!view.window) return 0;
    NSPoint p = [view convertPoint:view.window.mouseLocationOutsideOfEventStream fromView:nil];
    *x = p.x - view.bounds.origin.x;
    *y = view.isFlipped ? p.y - view.bounds.origin.y : NSMaxY(view.bounds) - p.y;
    return 1;
}

static const void *BGTrafficLightLayoutKey = &BGTrafficLightLayoutKey;
static const void *BGTitlebarDragViewKey = &BGTitlebarDragViewKey;
static const void *BGStudioVibrancyKey = &BGStudioVibrancyKey;

int bg_install_studio_vibrancy(void *view_pointer) {
    NSView *content = (__bridge NSView *)view_pointer;
    NSWindow *window = content.window;
    NSView *host = content.superview;
    if (!window || !host) return 0;

    NSVisualEffectView *effect = objc_getAssociatedObject(content, BGStudioVibrancyKey);
    NSRect contentFrame = [host convertRect:content.bounds fromView:content];
    if (!effect) {
        effect = [[NSVisualEffectView alloc] initWithFrame:contentFrame];
        effect.material = NSVisualEffectMaterialHeaderView;
        effect.blendingMode = NSVisualEffectBlendingModeBehindWindow;
        effect.state = NSVisualEffectStateFollowsWindowActiveState;
        effect.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        [host addSubview:effect positioned:NSWindowBelow relativeTo:content];
        objc_setAssociatedObject(content, BGStudioVibrancyKey, effect,
            OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    } else {
        effect.frame = contentFrame;
    }
    return 1;
}

@interface BGTitlebarDragView : NSView
@property CGFloat menuLeft;
@property CGFloat menuWidth;
@property CGFloat rightControlsWidth;
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
    if (x < 78.0 || (x >= self.menuLeft && x < self.menuLeft + self.menuWidth) || x >= self.bounds.size.width - self.rightControlsWidth) {
        return nil;
    }
    return self;
}
@end

void bg_install_titlebar_drag_region(void *view_pointer, double height, double menu_left, double menu_width, double right_width) {
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
    dragView.menuLeft = menu_left;
    dragView.menuWidth = menu_width;
    dragView.rightControlsWidth = right_width;
}

// AppKit may relayout standard buttons without a winit resize event (for
// example when changing key-window status or rebuilding the titlebar). The
// native content view owns this observer; no timer or saved button frames are
// needed. Positions are always derived from the current content coordinates.
@interface BGTrafficLightLayout : NSObject
@property (weak) NSView *content;
@property CGFloat left;
@property CGFloat headerHeight;
@property BOOL positioning;
@property BOOL layoutPending;
@property (strong) NSHashTable<NSView *> *trackedViews;
- (void)positionButtons;
@end

@implementation BGTrafficLightLayout
- (instancetype)init {
    self = [super init];
    if (self) {
        _trackedViews = [NSHashTable weakObjectsHashTable];
        NSNotificationCenter *center = NSNotificationCenter.defaultCenter;
        for (NSNotificationName name in @[
            NSViewFrameDidChangeNotification, NSViewBoundsDidChangeNotification,
            NSWindowDidResizeNotification, NSWindowDidBecomeKeyNotification,
            NSWindowDidResignKeyNotification, NSWindowDidDeminiaturizeNotification,
            NSWindowDidChangeBackingPropertiesNotification,
            NSWindowDidEnterFullScreenNotification, NSWindowDidExitFullScreenNotification
        ]) {
            [center addObserver:self selector:@selector(layoutChanged:) name:name object:nil];
        }
    }
    return self;
}
- (void)dealloc {
    [NSNotificationCenter.defaultCenter removeObserver:self];
}
- (void)layoutChanged:(NSNotification *)notification {
    if (notification.object == self.content.window ||
        [self.trackedViews containsObject:notification.object]) {
        if (self.positioning) return;
        [self positionButtons];
        // Frame notifications can arrive inside AppKit's layout transaction.
        // Reapply after that transaction has finished, including writes made
        // after the notification. Coalesce requests and keep ownership weak.
        if (!self.layoutPending) {
            self.layoutPending = YES;
            __weak BGTrafficLightLayout *weakSelf = self;
            dispatch_async(dispatch_get_main_queue(), ^{
                BGTrafficLightLayout *layout = weakSelf;
                if (!layout) return;
                layout.layoutPending = NO;
                [layout positionButtons];
            });
        }
    }
}
- (void)trackView:(NSView *)view {
    // Container ancestors can move without changing any button's local frame.
    for (; view; view = view.superview) {
        [self.trackedViews addObject:view];
        view.postsFrameChangedNotifications = YES;
        view.postsBoundsChangedNotifications = YES;
    }
}
- (void)positionButtons {
    NSView *content = self.content;
    NSWindow *window = content.window;
    if (!window || self.positioning) return;
    self.positioning = YES;
    [self.trackedViews removeAllObjects];
    [self trackView:content];
    CGFloat x = NSMinX(content.bounds) + self.left;
    CGFloat centerY = content.isFlipped
        ? NSMinY(content.bounds) + self.headerHeight * 0.5
        : NSMaxY(content.bounds) - self.headerHeight * 0.5;
    NSWindowButton kinds[] = { NSWindowCloseButton, NSWindowMiniaturizeButton, NSWindowZoomButton };
    for (NSUInteger index = 0; index < 3; index++) {
        NSButton *button = [window standardWindowButton:kinds[index]];
        NSView *container = button.superview;
        if (!button || !container) continue;
        [self trackView:button];
        NSPoint center = [container convertPoint:NSMakePoint(x + button.frame.size.width * 0.5, centerY)
            fromView:content];
        NSRect frame = button.frame;
        frame.origin = NSMakePoint(center.x - frame.size.width * 0.5,
            center.y - frame.size.height * 0.5);
        if (!NSEqualRects(frame, button.frame)) button.frame = frame;
        x += frame.size.width + 8.0;
    }
    self.positioning = NO;
}
@end

void bg_install_traffic_light_layout(void *view_pointer, double left, double header_height) {
    NSView *content = (__bridge NSView *)view_pointer;
    if (!content.window) return;
    BGTrafficLightLayout *layout = objc_getAssociatedObject(content, BGTrafficLightLayoutKey);
    if (!layout) {
        layout = [[BGTrafficLightLayout alloc] init];
        layout.content = content;
        objc_setAssociatedObject(content, BGTrafficLightLayoutKey, layout,
            OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    }
    layout.left = left;
    layout.headerHeight = header_height;
    [layout positionButtons];
}
