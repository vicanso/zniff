#import <AppKit/AppKit.h>
#import <Foundation/Foundation.h>
#import <NetworkExtension/NetworkExtension.h>
#import <SystemExtensions/SystemExtensions.h>
#include <stdbool.h>
#include <stdint.h>
#include <string.h>

static NSString *const kZniffSysextBundleId = @"dev.zniff.app.transparent-proxy";
static NSString *const kZniffProviderDescription = @"zniff Transparent Proxy";

static void zniff_write_cstr(char *buf, size_t len, NSString *text) {
    if (!buf || len == 0) {
        return;
    }
    const char *utf8 = text ? text.UTF8String : "";
    if (!utf8) {
        buf[0] = '\0';
        return;
    }
    strlcpy(buf, utf8, len);
}

static void zniff_write_json(char *buf, size_t len, id obj) {
    if (!buf || len == 0) {
        return;
    }
    NSError *err = nil;
    NSData *data = [NSJSONSerialization dataWithJSONObject:obj options:0 error:&err];
    if (!data || err) {
        buf[0] = '\0';
        return;
    }
    NSString *json = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
    zniff_write_cstr(buf, len, json);
}

static NSArray<NSString *> *zniff_parse_targets_json(const char *targets_json) {
    if (!targets_json || targets_json[0] == '\0') {
        return @[];
    }
    NSData *data = [NSData dataWithBytes:targets_json length:strlen(targets_json)];
    id parsed = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
    if (![parsed isKindOfClass:[NSArray class]]) {
        return @[];
    }
    NSMutableArray<NSString *> *out = [NSMutableArray array];
    for (id item in (NSArray *)parsed) {
        if ([item isKindOfClass:[NSString class]]) {
            NSString *trimmed = [(NSString *)item stringByTrimmingCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]];
            if (trimmed.length > 0) {
                [out addObject:trimmed];
            }
        }
    }
    return out;
}

static NSDictionary *zniff_read_app_plist(NSString *appPath) {
    if (appPath.length == 0) {
        return nil;
    }
    NSArray<NSString *> *candidates = @[
        [appPath stringByAppendingPathComponent:@"Contents/Info.plist"],
        [appPath stringByAppendingPathComponent:@"WrappedBundle/Info.plist"],
    ];
    for (NSString *plistPath in candidates) {
        NSDictionary *dict = [NSDictionary dictionaryWithContentsOfFile:plistPath];
        if ([dict isKindOfClass:[NSDictionary class]]) {
            return dict;
        }
    }
    return nil;
}

static void zniff_add_alias(NSMutableSet<NSString *> *aliases, NSString *value) {
    if (![value isKindOfClass:[NSString class]]) {
        return;
    }
    NSString *trimmed = [value stringByTrimmingCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]];
    if (trimmed.length > 0) {
        [aliases addObject:trimmed.lowercaseString];
    }
}

static void zniff_add_catalog_entry(
    NSMutableArray<NSDictionary *> *catalog,
    NSString *bundleId,
    NSString *displayName,
    NSString *execName,
    NSSet<NSString *> *aliases
) {
    if (bundleId.length == 0 || [bundleId isEqualToString:@"dev.zniff.app"]) {
        return;
    }
    NSString *label = displayName.length > 0 ? displayName : execName;
    if (label.length == 0) {
        label = bundleId;
    }
    [catalog addObject:@{
        @"bundleId": bundleId,
        @"displayName": label,
        @"execName": execName ?: @"",
        @"aliases": aliases,
    }];
}

static void zniff_scan_bundle_path(
    NSString *appPath,
    NSString *fallbackName,
    NSMutableArray<NSDictionary *> *catalog
) {
    NSDictionary *info = zniff_read_app_plist(appPath);
    if (!info) {
        return;
    }
    NSString *bundleId = [info[@"CFBundleIdentifier"] isKindOfClass:[NSString class]] ? info[@"CFBundleIdentifier"] : @"";
    if (bundleId.length == 0) {
        return;
    }
    NSString *cfDisplay = [info[@"CFBundleDisplayName"] isKindOfClass:[NSString class]] ? info[@"CFBundleDisplayName"] : @"";
    NSString *cfName = [info[@"CFBundleName"] isKindOfClass:[NSString class]] ? info[@"CFBundleName"] : @"";
    NSString *execName = [info[@"CFBundleExecutable"] isKindOfClass:[NSString class]] ? info[@"CFBundleExecutable"] : @"";
    NSString *folderName = appPath.lastPathComponent;
    if ([folderName.lowercaseString hasSuffix:@".app"] && folderName.length > 4) {
        folderName = [folderName substringToIndex:folderName.length - 4];
    }
    NSString *localizedDisplay = [NSFileManager.defaultManager displayNameAtPath:appPath] ?: @"";
    if ([localizedDisplay.lowercaseString hasSuffix:@".app"] && localizedDisplay.length > 4) {
        localizedDisplay = [localizedDisplay substringToIndex:localizedDisplay.length - 4];
    }

    NSString *displayName = fallbackName.length > 0 ? fallbackName : (localizedDisplay.length > 0 ? localizedDisplay : (cfDisplay.length > 0 ? cfDisplay : (cfName.length > 0 ? cfName : folderName)));

    NSMutableSet<NSString *> *aliases = [NSMutableSet set];
    zniff_add_alias(aliases, bundleId);
    zniff_add_alias(aliases, displayName);
    zniff_add_alias(aliases, localizedDisplay);
    zniff_add_alias(aliases, folderName);
    zniff_add_alias(aliases, cfDisplay);
    zniff_add_alias(aliases, cfName);
    zniff_add_alias(aliases, execName);
    zniff_add_catalog_entry(catalog, bundleId, displayName, execName, aliases);
}

static NSArray<NSDictionary *> *zniff_collect_app_catalog(void) {
    NSMutableArray<NSDictionary *> *catalog = [NSMutableArray array];
    NSMutableSet<NSString *> *seenBundleIds = [NSMutableSet set];

    for (NSRunningApplication *app in NSWorkspace.sharedWorkspace.runningApplications) {
        NSString *bundleId = app.bundleIdentifier ?: @"";
        if (bundleId.length == 0 || [bundleId isEqualToString:@"dev.zniff.app"]) {
            continue;
        }
        NSString *name = app.localizedName ?: @"";
        NSString *execName = app.executableURL.lastPathComponent ?: @"";
        NSString *bundlePath = app.bundleURL.path ?: @"";
        if (bundlePath.length > 0) {
            zniff_scan_bundle_path(bundlePath, name, catalog);
        }
        NSMutableSet<NSString *> *aliases = [NSMutableSet set];
        zniff_add_alias(aliases, bundleId);
        zniff_add_alias(aliases, name);
        zniff_add_alias(aliases, execName);
        zniff_add_catalog_entry(catalog, bundleId, name, execName, aliases);
        [seenBundleIds addObject:bundleId.lowercaseString];
    }

    NSFileManager *fm = NSFileManager.defaultManager;
    NSArray<NSString *> *roots = @[
        @"/Applications",
        @"/Applications/Utilities",
        @"/System/Applications",
        [NSHomeDirectory() stringByAppendingPathComponent:@"Applications"],
    ];
    for (NSString *root in roots) {
        NSArray<NSString *> *entries = [fm contentsOfDirectoryAtPath:root error:nil];
        for (NSString *entry in entries) {
            if (![entry.lowercaseString hasSuffix:@".app"]) {
                continue;
            }
            NSString *fullPath = [root stringByAppendingPathComponent:entry];
            NSDictionary *info = zniff_read_app_plist(fullPath);
            NSString *bid = [info[@"CFBundleIdentifier"] isKindOfClass:[NSString class]] ? info[@"CFBundleIdentifier"] : @"";
            if (bid.length > 0 && ![seenBundleIds containsObject:bid.lowercaseString]) {
                [seenBundleIds addObject:bid.lowercaseString];
                zniff_scan_bundle_path(fullPath, @"", catalog);
            } else if (bid.length > 0) {
                zniff_scan_bundle_path(fullPath, @"", catalog);
            }
        }
    }
    return catalog;
}

static NSDictionary *zniff_build_provider_config(
    uint16_t port,
    const char *proxy_auth,
    bool capture_all,
    const char *targets_json,
    bool block_quic
) {
    NSString *auth = proxy_auth ? [NSString stringWithUTF8String:proxy_auth] : @"";
    if (!auth) auth = @"";
    NSArray<NSString *> *rawTargets = zniff_parse_targets_json(targets_json);
    NSArray<NSDictionary *> *catalog = zniff_collect_app_catalog();

    NSMutableArray<NSString *> *expandedTargets = [NSMutableArray array];
    NSMutableSet<NSString *> *seenTargets = [NSMutableSet set];
    void (^appendTarget)(NSString *) = ^(NSString *val) {
        NSString *trimmed = [val stringByTrimmingCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]];
        if (trimmed.length == 0) return;
        NSString *lower = trimmed.lowercaseString;
        if (![seenTargets containsObject:lower]) {
            [seenTargets addObject:lower];
            [expandedTargets addObject:trimmed];
        }
    };

    for (NSString *raw in rawTargets) {
        appendTarget(raw);
        NSString *needle = raw.lowercaseString;
        for (NSDictionary *entry in catalog) {
            NSSet<NSString *> *aliases = entry[@"aliases"];
            if ([aliases containsObject:needle]) {
                appendTarget(entry[@"bundleId"]);
                appendTarget(entry[@"execName"]);
            }
        }
    }

    NSMutableDictionary<NSString *, NSString *> *appLabels = [NSMutableDictionary dictionary];
    for (NSDictionary *entry in catalog) {
        if (appLabels.count >= 512) {
            break;
        }
        NSString *bid = [(NSString *)entry[@"bundleId"] lowercaseString];
        NSString *label = entry[@"displayName"];
        if (bid.length > 0 && label.length > 0 && !appLabels[bid]) {
            appLabels[bid] = label;
        }
    }

    return @{
        @"proxyPort": @(port),
        @"proxyAuth": auth,
        @"captureAll": @(capture_all),
        @"targets": expandedTargets,
        @"appLabels": appLabels,
        @"blockQuic": @(block_quic),
    };
}

int zniff_tp_resolve_targets_json(const char *targets_json, char *buf, size_t len) {
    @autoreleasepool {
        NSDictionary *cfg = zniff_build_provider_config(8877, "", false, targets_json, true);
        zniff_write_json(buf, len, @{
            @"targets": cfg[@"targets"] ?: @[],
            @"appLabels": cfg[@"appLabels"] ?: @{},
        });
        return 0;
    }
}

@interface ZniffTpSharedState : NSObject
@property (nonatomic, strong) NSLock *lock;
@property (nonatomic, assign) BOOL activating;
@property (nonatomic, assign) BOOL needsApproval;
@property (nonatomic, copy) NSString *lastError;
@property (nonatomic, strong) NSDictionary *pendingConfig;
@property (nonatomic, strong) id activeDelegate;
+ (instancetype)shared;
@end

@implementation ZniffTpSharedState
+ (instancetype)shared {
    static ZniffTpSharedState *inst = nil;
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        inst = [[ZniffTpSharedState alloc] init];
        inst.lock = [[NSLock alloc] init];
        inst.lastError = @"";
    });
    return inst;
}
@end

static void zniff_configure_and_start_manager(
    NSDictionary *providerConfig,
    void (^completion)(int code, NSString *errMsg)
) {
    [NETransparentProxyManager loadAllFromPreferencesWithCompletionHandler:^(NSArray<NETransparentProxyManager *> * _Nullable managers, NSError * _Nullable error) {
        if (error) {
            completion(-1, error.localizedDescription ?: @"loadAllFromPreferences failed");
            return;
        }
        NETransparentProxyManager *target = nil;
        for (NETransparentProxyManager *mgr in managers) {
            if ([mgr.protocolConfiguration isKindOfClass:[NETunnelProviderProtocol class]]) {
                NETunnelProviderProtocol *proto = (NETunnelProviderProtocol *)mgr.protocolConfiguration;
                if ([proto.providerBundleIdentifier isEqualToString:kZniffSysextBundleId]) {
                    target = mgr;
                    break;
                }
            }
        }
        if (!target) {
            target = [[NETransparentProxyManager alloc] init];
        }

        NETunnelProviderProtocol *proto = [[NETunnelProviderProtocol alloc] init];
        proto.providerBundleIdentifier = kZniffSysextBundleId;
        proto.serverAddress = @"127.0.0.1";
        proto.providerConfiguration = providerConfig;

        target.localizedDescription = kZniffProviderDescription;
        target.protocolConfiguration = proto;
        target.enabled = YES;

        [target saveToPreferencesWithCompletionHandler:^(NSError * _Nullable saveErr) {
            if (saveErr) {
                completion(-1, saveErr.localizedDescription ?: @"saveToPreferences failed");
                return;
            }
            [target loadFromPreferencesWithCompletionHandler:^(NSError * _Nullable loadErr) {
                if (loadErr) {
                    completion(-1, loadErr.localizedDescription ?: @"loadFromPreferences failed");
                    return;
                }
                NSError *startErr = nil;
                NEVPNStatus status = target.connection.status;
                if (status == NEVPNStatusConnected || status == NEVPNStatusConnecting) {
                    NETunnelProviderSession *session = (NETunnelProviderSession *)target.connection;
                    NSData *msgData = [NSJSONSerialization dataWithJSONObject:providerConfig options:0 error:nil];
                    if (msgData) {
                        [session sendProviderMessage:msgData returnError:nil responseHandler:nil];
                    }
                    completion(0, @"");
                    return;
                }
                BOOL ok = [target.connection startVPNTunnelAndReturnError:&startErr];
                if (!ok || startErr) {
                    completion(-1, startErr.localizedDescription ?: @"startVPNTunnel failed");
                    return;
                }
                completion(0, @"");
            }];
        }];
    }];
}

@interface ZniffSysextActivationDelegate : NSObject <OSSystemExtensionRequestDelegate>
@property (nonatomic, strong) dispatch_semaphore_t sem;
@property (nonatomic, assign) int resultCode;
@property (nonatomic, copy) NSString *errorMessage;
@property (nonatomic, assign) BOOL signaled;
@end

@implementation ZniffSysextActivationDelegate

- (instancetype)initWithSemaphore:(dispatch_semaphore_t)sem {
    self = [super init];
    if (self) {
        _sem = sem;
        _resultCode = -1;
        _errorMessage = @"";
        _signaled = NO;
    }
    return self;
}

- (void)signalOnceWithCode:(int)code error:(NSString *)errMsg {
    @synchronized (self) {
        if (_signaled) {
            return;
        }
        _signaled = YES;
        _resultCode = code;
        _errorMessage = errMsg ?: @"";
        dispatch_semaphore_signal(_sem);
    }
}

- (OSSystemExtensionReplacementAction)request:(OSSystemExtensionRequest *)request
                  actionForReplacingExtension:(OSSystemExtensionProperties *)existing
                                withExtension:(OSSystemExtensionProperties *)ext {
    return OSSystemExtensionReplacementActionReplace;
}

- (void)requestNeedsUserApproval:(OSSystemExtensionRequest *)request {
    ZniffTpSharedState *state = [ZniffTpSharedState shared];
    [state.lock lock];
    state.needsApproval = YES;
    state.activating = NO;
    state.lastError = @"";
    [state.lock unlock];
    [self signalOnceWithCode:2 error:@""];
}

- (void)request:(OSSystemExtensionRequest *)request didFinishWithResult:(OSSystemExtensionRequestResult)result {
    ZniffTpSharedState *state = [ZniffTpSharedState shared];
    [state.lock lock];
    state.needsApproval = NO;
    state.activating = NO;
    NSDictionary *cfg = state.pendingConfig;
    state.activeDelegate = nil;
    [state.lock unlock];

    if (!cfg) {
        [self signalOnceWithCode:0 error:@""];
        return;
    }
    zniff_configure_and_start_manager(cfg, ^(int code, NSString *errMsg) {
        [state.lock lock];
        state.lastError = errMsg ?: @"";
        [state.lock unlock];
        [self signalOnceWithCode:code error:errMsg];
    });
}

- (void)request:(OSSystemExtensionRequest *)request didFailWithError:(NSError *)error {
    NSString *msg = error.localizedDescription ?: @"OSSystemExtensionRequest failed";
    ZniffTpSharedState *state = [ZniffTpSharedState shared];
    [state.lock lock];
    state.needsApproval = NO;
    state.activating = NO;
    state.lastError = msg;
    state.activeDelegate = nil;
    [state.lock unlock];
    [self signalOnceWithCode:-1 error:msg];
}

@end

int zniff_tp_bundle_status(char *buf, size_t len) {
    @autoreleasepool {
        NSString *bundlePath = NSBundle.mainBundle.bundlePath ?: @"";
        NSString *sysextPath = [bundlePath stringByAppendingPathComponent:@"Contents/Library/SystemExtensions/dev.zniff.app.transparent-proxy.systemextension"];
        BOOL embedded = [[NSFileManager defaultManager] fileExistsAtPath:sysextPath];
        BOOL inApplications = [bundlePath hasPrefix:@"/Applications/"];
        zniff_write_json(buf, len, @{
            @"embedded": @(embedded),
            @"inApplications": @(inApplications),
            @"bundlePath": bundlePath,
        });
        return 0;
    }
}

int zniff_tp_get_status(char *buf, size_t len) {
    @autoreleasepool {
        ZniffTpSharedState *shared = [ZniffTpSharedState shared];
        [shared.lock lock];
        BOOL activating = shared.activating;
        BOOL needsApproval = shared.needsApproval;
        NSString *lastErr = [shared.lastError copy] ?: @"";
        [shared.lock unlock];

        __block NSString *stateStr = @"idle";
        __block BOOL active = NO;
        __block NSString *errStr = lastErr;

        dispatch_semaphore_t sem = dispatch_semaphore_create(0);
        [NETransparentProxyManager loadAllFromPreferencesWithCompletionHandler:^(NSArray<NETransparentProxyManager *> * _Nullable managers, NSError * _Nullable error) {
            if (!error) {
                for (NETransparentProxyManager *mgr in managers) {
                    if ([mgr.protocolConfiguration isKindOfClass:[NETunnelProviderProtocol class]]) {
                        NETunnelProviderProtocol *proto = (NETunnelProviderProtocol *)mgr.protocolConfiguration;
                        if ([proto.providerBundleIdentifier isEqualToString:kZniffSysextBundleId]) {
                            NEVPNStatus st = mgr.connection.status;
                            if (st == NEVPNStatusConnected) {
                                stateStr = @"running";
                                active = YES;
                            } else if (st == NEVPNStatusConnecting || st == NEVPNStatusReasserting) {
                                stateStr = @"starting";
                                active = YES;
                            } else if (st == NEVPNStatusDisconnecting) {
                                stateStr = @"stopping";
                                active = YES;
                            }
                            break;
                        }
                    }
                }
            }
            dispatch_semaphore_signal(sem);
        }];
        dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(3 * NSEC_PER_SEC)));

        if (!active) {
            if (needsApproval) {
                stateStr = @"needs_approval";
            } else if (activating) {
                stateStr = @"starting";
            } else if (errStr.length > 0) {
                stateStr = @"error";
            }
        }
        zniff_write_json(buf, len, @{
            @"state": stateStr,
            @"active": @(active),
            @"error": errStr,
        });
        return 0;
    }
}

int zniff_tp_activate_and_start(
    uint16_t port,
    const char *proxy_auth,
    bool capture_all,
    const char *targets_json,
    bool block_quic,
    char *err_buf,
    size_t err_len
) {
    @autoreleasepool {
        NSDictionary *cfg = zniff_build_provider_config(port, proxy_auth, capture_all, targets_json, block_quic);
        ZniffTpSharedState *shared = [ZniffTpSharedState shared];
        dispatch_semaphore_t sem = dispatch_semaphore_create(0);
        ZniffSysextActivationDelegate *delegate = [[ZniffSysextActivationDelegate alloc] initWithSemaphore:sem];

        [shared.lock lock];
        shared.activating = YES;
        shared.needsApproval = NO;
        shared.lastError = @"";
        shared.pendingConfig = cfg;
        shared.activeDelegate = delegate;
        [shared.lock unlock];

        dispatch_queue_t q = dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0);
        OSSystemExtensionRequest *req = [OSSystemExtensionRequest activationRequestForExtension:kZniffSysextBundleId queue:q];
        req.delegate = delegate;
        [[OSSystemExtensionManager sharedManager] submitRequest:req];

        long waited = dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(30 * NSEC_PER_SEC)));
        if (waited != 0) {
            zniff_write_cstr(err_buf, err_len, @"Timed out waiting for system extension activation");
            return -1;
        }
        if (delegate.resultCode < 0) {
            zniff_write_cstr(err_buf, err_len, delegate.errorMessage);
        }
        return delegate.resultCode;
    }
}

int zniff_tp_update_config(
    uint16_t port,
    const char *proxy_auth,
    bool capture_all,
    const char *targets_json,
    bool block_quic
) {
    @autoreleasepool {
        NSDictionary *cfg = zniff_build_provider_config(port, proxy_auth, capture_all, targets_json, block_quic);
        ZniffTpSharedState *shared = [ZniffTpSharedState shared];
        [shared.lock lock];
        shared.pendingConfig = cfg;
        [shared.lock unlock];

        __block int rc = 0;
        dispatch_semaphore_t sem = dispatch_semaphore_create(0);
        [NETransparentProxyManager loadAllFromPreferencesWithCompletionHandler:^(NSArray<NETransparentProxyManager *> * _Nullable managers, NSError * _Nullable error) {
            if (error) {
                rc = -1;
                dispatch_semaphore_signal(sem);
                return;
            }
            NETransparentProxyManager *target = nil;
            for (NETransparentProxyManager *mgr in managers) {
                if ([mgr.protocolConfiguration isKindOfClass:[NETunnelProviderProtocol class]]) {
                    NETunnelProviderProtocol *proto = (NETunnelProviderProtocol *)mgr.protocolConfiguration;
                    if ([proto.providerBundleIdentifier isEqualToString:kZniffSysextBundleId]) {
                        target = mgr;
                        break;
                    }
                }
            }
            if (!target) {
                dispatch_semaphore_signal(sem);
                return;
            }
            NETunnelProviderProtocol *proto = (NETunnelProviderProtocol *)target.protocolConfiguration;
            proto.providerConfiguration = cfg;
            target.protocolConfiguration = proto;
            [target saveToPreferencesWithCompletionHandler:^(NSError * _Nullable saveErr) {
                if (saveErr) {
                    rc = -1;
                }
                NEVPNStatus st = target.connection.status;
                if (st == NEVPNStatusConnected || st == NEVPNStatusConnecting) {
                    NETunnelProviderSession *session = (NETunnelProviderSession *)target.connection;
                    NSData *msgData = [NSJSONSerialization dataWithJSONObject:cfg options:0 error:nil];
                    if (msgData) {
                        [session sendProviderMessage:msgData returnError:nil responseHandler:nil];
                    }
                }
                dispatch_semaphore_signal(sem);
            }];
        }];
        dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(5 * NSEC_PER_SEC)));
        return rc;
    }
}

int zniff_tp_stop(void) {
    @autoreleasepool {
        __block int rc = 0;
        dispatch_semaphore_t sem = dispatch_semaphore_create(0);
        [NETransparentProxyManager loadAllFromPreferencesWithCompletionHandler:^(NSArray<NETransparentProxyManager *> * _Nullable managers, NSError * _Nullable error) {
            if (error) {
                rc = -1;
                dispatch_semaphore_signal(sem);
                return;
            }
            for (NETransparentProxyManager *mgr in managers) {
                if ([mgr.protocolConfiguration isKindOfClass:[NETunnelProviderProtocol class]]) {
                    NETunnelProviderProtocol *proto = (NETunnelProviderProtocol *)mgr.protocolConfiguration;
                    if ([proto.providerBundleIdentifier isEqualToString:kZniffSysextBundleId]) {
                        [mgr.connection stopVPNTunnel];
                    }
                }
            }
            dispatch_semaphore_signal(sem);
        }];
        dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(5 * NSEC_PER_SEC)));
        return rc;
    }
}

int zniff_tp_list_running_apps(char *buf, size_t len) {
    @autoreleasepool {
        NSArray<NSRunningApplication *> *apps = NSWorkspace.sharedWorkspace.runningApplications;
        NSMutableDictionary<NSString *, NSDictionary *> *byKey = [NSMutableDictionary dictionary];
        for (NSRunningApplication *app in apps) {
            if (app.activationPolicy != NSApplicationActivationPolicyRegular) {
                continue;
            }
            NSString *bundleId = app.bundleIdentifier ?: @"";
            if ([bundleId isEqualToString:@"dev.zniff.app"]) {
                continue;
            }
            NSString *name = app.localizedName ?: @"";
            NSString *execName = app.executableURL.lastPathComponent ?: name;
            if (name.length == 0 && execName.length == 0) {
                continue;
            }
            if (name.length == 0) name = execName;
            NSString *key = bundleId.length > 0 ? bundleId : name;
            if (!byKey[key]) {
                byKey[key] = @{
                    @"name": name,
                    @"bundleId": bundleId,
                    @"execName": execName,
                };
            }
        }
        NSArray<NSDictionary *> *sorted = [byKey.allValues sortedArrayUsingComparator:^NSComparisonResult(NSDictionary *a, NSDictionary *b) {
            return [a[@"name"] localizedCaseInsensitiveCompare:b[@"name"]];
        }];
        zniff_write_json(buf, len, sorted);
        return 0;
    }
}

void zniff_tp_open_system_settings(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        NSURL *url = [NSURL URLWithString:@"x-apple.systempreferences:com.apple.LoginItems-Settings.extension"];
        if (!url || ![NSWorkspace.sharedWorkspace openURL:url]) {
            NSURL *fallback = [NSURL URLWithString:@"x-apple.systempreferences:com.apple.preference.security?Privacy"];
            if (fallback) {
                [NSWorkspace.sharedWorkspace openURL:fallback];
            }
        }
    });
}
