//! Android 7+ (API 24+) user certificate and `network_security_config.xml` guidance.
//!
//! Starting with Android 7.0, apps ignore user-installed CA certificates by default.
//! This module provides ready-to-paste XML snippets, manifest attributes, emulator/root
//! system-cert commands, and the HTML section embedded in the `/zniff` mobile install page.

use crate::i18n::Lang;

/// Ready-to-paste `res/xml/network_security_config.xml` allowing user-installed CAs in debug builds.
pub const NETWORK_SECURITY_CONFIG_XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<network-security-config>
    <debug-overrides>
        <trust-anchors>
            <certificates src="user" />
            <certificates src="system" />
        </trust-anchors>
    </debug-overrides>
</network-security-config>"#;

/// Attribute to add on `<application>` in `AndroidManifest.xml`.
pub const MANIFEST_APPLICATION_ATTR: &str =
    r#"android:networkSecurityConfig="@xml/network_security_config""#;

/// Shell commands for installing `zniff-ca.crt` as a system CA on a writable Android emulator
/// (`emulator -writable-system`) or rooted device.
pub const EMULATOR_SYSTEM_CERT_COMMANDS: &str = r#"HASH=$(openssl x509 -inform PEM -subject_hash_old -in zniff-ca.crt | head -1)
adb root && adb remount
adb push zniff-ca.crt "/system/etc/security/cacerts/${HASH}.0"
adb shell "chmod 644 /system/etc/security/cacerts/${HASH}.0"
adb reboot"#;

/// Compact preview shown inside the desktop Connect Hub collapsible panel.
pub fn connect_hub_preview() -> &'static str {
    concat!(
        "<!-- res/xml/network_security_config.xml -->\n",
        "<network-security-config>\n",
        "  <debug-overrides>\n",
        "    <trust-anchors>\n",
        "      <certificates src=\"user\" />\n",
        "      <certificates src=\"system\" />\n",
        "    </trust-anchors>\n",
        "  </debug-overrides>\n",
        "</network-security-config>"
    )
}

/// Returns the complete bilingual guide text suitable for copying to the clipboard.
pub fn full_clipboard_guide(lang: Lang) -> String {
    match lang {
        Lang::Zh => format!(
            "# Android 7+ 用户证书抓包配置\n\n\
             Android 7.0 (API 24) 起，应用默认不信任用户安装的 CA 证书。\n\n\
             ## 1. Debug 构建（推荐）\n\
             在项目中创建 `res/xml/network_security_config.xml`：\n\
             ```xml\n{NETWORK_SECURITY_CONFIG_XML}\n```\n\n\
             在 `AndroidManifest.xml` 的 `<application>` 节点添加：\n\
             ```xml\n{MANIFEST_APPLICATION_ATTR}\n```\n\n\
             ## 2. 模拟器 / Root 设备写入系统证书（非 Debug 包）\n\
             使用 `-writable-system` 启动模拟器或具备 root 权限后执行：\n\
             ```sh\n{EMULATOR_SYSTEM_CERT_COMMANDS}\n```\n\
             注：Android 14+ (API 34+) 系统证书位于只读 APEX (`/apex/com.android.conscrypt/cacerts`)，需通过 tmpfs 挂载或 Magisk 模块注入。\n"
        ),
        Lang::En => format!(
            "# Android 7+ User Certificate Capture Guide\n\n\
             Starting with Android 7.0 (API 24), apps ignore user-installed CA certificates by default.\n\n\
             ## 1. Debug builds (recommended)\n\
             Create `res/xml/network_security_config.xml` in your Android app:\n\
             ```xml\n{NETWORK_SECURITY_CONFIG_XML}\n```\n\n\
             Add this attribute to `<application>` in `AndroidManifest.xml`:\n\
             ```xml\n{MANIFEST_APPLICATION_ATTR}\n```\n\n\
             ## 2. Emulator / rooted device system CA (release / non-debuggable APKs)\n\
             Start the emulator with `-writable-system` (or use a rooted device) and run:\n\
             ```sh\n{EMULATOR_SYSTEM_CERT_COMMANDS}\n```\n\
             Note: On Android 14+ (API 34+), system CAs live in a read-only APEX (`/apex/com.android.conscrypt/cacerts`) and require a tmpfs overlay or Magisk module.\n"
        ),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 32);
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Renders the Android 7+ guidance `<details>` section embedded into `/zniff`.
#[cfg(not(target_arch = "wasm32"))]
pub fn install_page_android_section(zh: bool) -> String {
    let xml_escaped = escape_html(NETWORK_SECURITY_CONFIG_XML);
    let manifest_escaped = escape_html(MANIFEST_APPLICATION_ATTR);
    let emu_escaped = escape_html(EMULATOR_SYSTEM_CERT_COMMANDS);

    if zh {
        format!(
            r#"  <details class="card" open>
    <summary>Android 7+ 应用抓包配置（<code>network_security_config.xml</code>）</summary>
    <p class="note">Android 7.0 (API 24) 起，App 默认忽略用户安装的 CA 证书。调试自有 App 时请添加以下配置，仅对 Debug 构建生效：</p>
    <div class="snippet">
      <div class="snippet-head">
        <span><code>res/xml/network_security_config.xml</code></span>
        <button type="button" class="copy-btn" data-copy="nsc-xml" data-ok="已复制">复制 XML</button>
      </div>
      <pre><code id="nsc-xml">{xml_escaped}</code></pre>
    </div>
    <div class="snippet">
      <div class="snippet-head">
        <span><code>AndroidManifest.xml</code>（<code>&lt;application&gt;</code> 属性）</span>
        <button type="button" class="copy-btn" data-copy="nsc-manifest" data-ok="已复制">复制</button>
      </div>
      <pre><code id="nsc-manifest">{manifest_escaped}</code></pre>
    </div>
    <details class="sub-card">
      <summary>模拟器 / Root 设备写入系统证书</summary>
      <p class="note">如需抓非 Debug 包，用 <code>-writable-system</code> 启动模拟器或使用 Root 设备执行：</p>
      <div class="snippet">
        <div class="snippet-head">
          <span>adb / openssl</span>
          <button type="button" class="copy-btn" data-copy="nsc-emu" data-ok="已复制">复制命令</button>
        </div>
        <pre><code id="nsc-emu">{emu_escaped}</code></pre>
      </div>
      <p class="note">Android 14+ (API 34+) 系统证书位于只读 APEX（<code>/apex/com.android.conscrypt/cacerts</code>），需通过 tmpfs 挂载或 Magisk 模块注入。</p>
    </details>
  </details>"#
        )
    } else {
        format!(
            r#"  <details class="card" open>
    <summary>Android 7+ app capture setup (<code>network_security_config.xml</code>)</summary>
    <p class="note">Android 7.0 (API 24)+ apps ignore user-installed CA certs by default. Add this config to trust user CAs in debug builds only:</p>
    <div class="snippet">
      <div class="snippet-head">
        <span><code>res/xml/network_security_config.xml</code></span>
        <button type="button" class="copy-btn" data-copy="nsc-xml" data-ok="Copied">Copy XML</button>
      </div>
      <pre><code id="nsc-xml">{xml_escaped}</code></pre>
    </div>
    <div class="snippet">
      <div class="snippet-head">
        <span><code>AndroidManifest.xml</code> (<code>&lt;application&gt;</code> attribute)</span>
        <button type="button" class="copy-btn" data-copy="nsc-manifest" data-ok="Copied">Copy</button>
      </div>
      <pre><code id="nsc-manifest">{manifest_escaped}</code></pre>
    </div>
    <details class="sub-card">
      <summary>Emulator / rooted device system certificate</summary>
      <p class="note">For non-debuggable APKs, launch the emulator with <code>-writable-system</code> (or use a rooted device) and run:</p>
      <div class="snippet">
        <div class="snippet-head">
          <span>adb / openssl</span>
          <button type="button" class="copy-btn" data-copy="nsc-emu" data-ok="Copied">Copy commands</button>
        </div>
        <pre><code id="nsc-emu">{emu_escaped}</code></pre>
      </div>
      <p class="note">Android 14+ (API 34+) stores system CAs in a read-only APEX (<code>/apex/com.android.conscrypt/cacerts</code>); use a tmpfs overlay or Magisk module.</p>
    </details>
  </details>"#
        )
    }
}

/// Inline copy helper script for `/zniff` that works on both HTTPS and plain-HTTP LAN origins.
#[cfg(not(target_arch = "wasm32"))]
pub const INSTALL_PAGE_COPY_SCRIPT: &str = r#"<script>
document.addEventListener('click', function (e) {
  var btn = e.target.closest('button.copy-btn');
  if (!btn) return;
  var el = document.getElementById(btn.getAttribute('data-copy'));
  if (!el) return;
  var text = el.textContent || '';
  var prev = btn.textContent;
  var ok = btn.getAttribute('data-ok') || 'Copied';
  function done() {
    btn.textContent = ok;
    btn.classList.add('copied');
    setTimeout(function () {
      btn.classList.remove('copied');
      btn.textContent = prev;
    }, 1600);
  }
  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(text).then(done).catch(fallback);
  } else {
    fallback();
  }
  function fallback() {
    var ta = document.createElement('textarea');
    ta.value = text;
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.select();
    try { document.execCommand('copy'); done(); } catch (_) {}
    document.body.removeChild(ta);
  }
});
</script>"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_snippet_is_well_formed_and_uses_debug_overrides() {
        assert!(NETWORK_SECURITY_CONFIG_XML.starts_with("<?xml version=\"1.0\""));
        assert!(NETWORK_SECURITY_CONFIG_XML.contains("<network-security-config>"));
        assert!(NETWORK_SECURITY_CONFIG_XML.contains("</network-security-config>"));
        assert!(NETWORK_SECURITY_CONFIG_XML.contains("<debug-overrides>"));
        assert!(NETWORK_SECURITY_CONFIG_XML.contains("</debug-overrides>"));
        assert!(NETWORK_SECURITY_CONFIG_XML.contains("<trust-anchors>"));
        assert!(NETWORK_SECURITY_CONFIG_XML.contains("</trust-anchors>"));
        assert!(NETWORK_SECURITY_CONFIG_XML.contains("<certificates src=\"user\" />"));
        assert!(NETWORK_SECURITY_CONFIG_XML.contains("<certificates src=\"system\" />"));
    }

    #[test]
    fn full_guide_includes_all_sections_in_both_languages() {
        for lang in [Lang::Zh, Lang::En] {
            let guide = full_clipboard_guide(lang);
            assert!(guide.contains(NETWORK_SECURITY_CONFIG_XML));
            assert!(guide.contains(MANIFEST_APPLICATION_ATTR));
            assert!(guide.contains("subject_hash_old"));
            assert!(guide.contains("/system/etc/security/cacerts/"));
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn install_page_section_escapes_xml_and_includes_copy_buttons() {
        for zh in [true, false] {
            let html = install_page_android_section(zh);
            assert!(html.contains("network_security_config.xml"));
            assert!(html.contains("&lt;network-security-config&gt;"));
            assert!(html.contains("&lt;certificates src=&quot;user&quot; /&gt;"));
            assert!(html.contains("data-copy=\"nsc-xml\""));
            assert!(html.contains("data-copy=\"nsc-manifest\""));
            assert!(html.contains("data-copy=\"nsc-emu\""));
        }
    }
}
