//! iOS physical device step-by-step certificate verification UI for `/zniff`.
//!
//! Renders an interactive checklist ("Profile installed" -> "Full Trust enabled in
//! Certificate Trust Settings") that probes `https://zniff.local/probe` through the
//! zniff MITM proxy and checks off automatically once TLS interception succeeds.

/// HTTPS probe URL intercepted locally by zniff's MITM handler (`serve_tls_probe`).
pub const IOS_TLS_PROBE_URL: &str = "https://zniff.local/probe";

/// Extra CSS rules for the iOS verification checklist on `/zniff`.
pub const IOS_CHECKLIST_STYLE: &str = r#"  .checklist { list-style: none; margin: 0.6rem 0; padding: 0; }
  .checklist li { display: flex; align-items: flex-start; gap: 0.55rem; padding: 0.5rem 0; border-bottom: 1px solid var(--card-border); font-size: 0.92rem; }
  .checklist li:last-child { border-bottom: none; }
  .step-icon { width: 1.25rem; height: 1.25rem; border-radius: 999px; border: 1.5px solid var(--copy-border); color: var(--muted); display: inline-flex; align-items: center; justify-content: center; font-size: 0.78rem; font-weight: 700; flex-shrink: 0; margin-top: 0.1rem; transition: background-color 160ms ease, border-color 160ms ease, color 160ms ease; }
  .checklist li.done .step-icon { background: #16a34a; border-color: #16a34a; color: #fff; }
  .checklist li.done .step-title { color: var(--ok-fg); }
  .probe-bar { display: flex; align-items: center; justify-content: space-between; gap: 0.6rem; margin-top: 0.65rem; flex-wrap: wrap; }
  .probe-status { font-size: 0.85rem; color: var(--muted); flex: 1; min-width: 12rem; }
  .probe-status.ok { color: var(--ok-fg); }
  .probe-status.fail { color: var(--err-fg); }
  .probe-btn { min-height: 40px; border: 1px solid var(--btn-bg); background: var(--btn-bg); color: #fff; border-radius: 8px; padding: 0.5rem 0.95rem; font-size: 0.85rem; font-weight: 600; cursor: pointer; touch-action: manipulation; transition: transform 100ms cubic-bezier(0.22, 1, 0.36, 1), opacity 100ms ease; }
  .probe-btn:active { transform: scale(0.97); opacity: 0.88; }"#;

/// Renders the bilingual iOS verification checklist card embedded into `/zniff`.
pub fn install_page_ios_checklist(zh: bool) -> String {
    if zh {
        format!(
            r#"  <section class="card" id="ios-verify" data-probe="{IOS_TLS_PROBE_URL}" data-checking="正在探测 {IOS_TLS_PROBE_URL} …" data-ok="✓ HTTPS 证书已完全信任，可以开始抓包" data-fail="尚未通过 HTTPS 探测：请确认 Wi-Fi 代理指向 zniff，且已在「设置 → 通用 → 关于本机 → 证书信任设置」开启 zniff CA 完全信任。">
    <div style="font-weight:600;font-size:0.95rem;">iOS 证书生效分步检测</div>
    <ul class="checklist">
      <li id="step-profile"><span class="step-icon" aria-hidden="true">1</span><div><div class="step-title">已下载并安装描述文件</div><div class="note">点上方「下载根证书」并在系统设置中完成安装</div></div></li>
      <li id="step-trust"><span class="step-icon" aria-hidden="true">2</span><div><div class="step-title">已在「证书信任设置」开启完全信任</div><div class="note">回连 <code>{IOS_TLS_PROBE_URL}</code> 自动验证 TLS 解密</div></div></li>
    </ul>
    <div class="probe-bar">
      <span class="probe-status" id="ios-probe-status">正在探测 {IOS_TLS_PROBE_URL} …</span>
      <button type="button" class="probe-btn" id="ios-probe-btn">重新检测</button>
    </div>
  </section>"#
        )
    } else {
        format!(
            r#"  <section class="card" id="ios-verify" data-probe="{IOS_TLS_PROBE_URL}" data-checking="Probing {IOS_TLS_PROBE_URL}…" data-ok="✓ HTTPS certificate is fully trusted — ready to capture" data-fail="HTTPS probe not yet trusted: ensure Wi-Fi proxy points to zniff and Full Trust is enabled in Settings → General → About → Certificate Trust Settings.">
    <div style="font-weight:600;font-size:0.95rem;">iOS Certificate Verification</div>
    <ul class="checklist">
      <li id="step-profile"><span class="step-icon" aria-hidden="true">1</span><div><div class="step-title">Profile installed</div><div class="note">Tap Download above and install zniff CA in Settings</div></div></li>
      <li id="step-trust"><span class="step-icon" aria-hidden="true">2</span><div><div class="step-title">Full Trust enabled in Certificate Trust Settings</div><div class="note">Probes <code>{IOS_TLS_PROBE_URL}</code> to confirm TLS interception</div></div></li>
    </ul>
    <div class="probe-bar">
      <span class="probe-status" id="ios-probe-status">Probing {IOS_TLS_PROBE_URL}…</span>
      <button type="button" class="probe-btn" id="ios-probe-btn">Verify HTTPS</button>
    </div>
  </section>"#
        )
    }
}

/// Inline script that runs the HTTPS probe on load, on button click, and when returning from Settings.
pub const IOS_PROBE_SCRIPT: &str = r#"<script>
(function () {
  var box = document.getElementById('ios-verify');
  if (!box) return;
  var probeUrl = box.getAttribute('data-probe') || 'https://zniff.local/probe';
  var stepProfile = document.getElementById('step-profile');
  var stepTrust = document.getElementById('step-trust');
  var statusEl = document.getElementById('ios-probe-status');
  var btn = document.getElementById('ios-probe-btn');
  var dl = document.querySelector('a.btn[href="/zniff-ca.crt"]');

  function markDone(el, num) {
    if (!el) return;
    el.classList.add('done');
    var icon = el.querySelector('.step-icon');
    if (icon) icon.textContent = '✓';
    el.setAttribute('data-step', String(num));
  }

  function markPending(el, num) {
    if (!el) return;
    el.classList.remove('done');
    var icon = el.querySelector('.step-icon');
    if (icon) icon.textContent = String(num);
  }

  if (dl) {
    dl.addEventListener('click', function () { markDone(stepProfile, 1); });
  }

  function runProbe() {
    if (statusEl) {
      statusEl.className = 'probe-status';
      statusEl.textContent = box.getAttribute('data-checking') || '';
    }
    fetch(probeUrl, { mode: 'cors', cache: 'no-store' })
      .then(function (res) {
        if (!res.ok) throw new Error('status ' + res.status);
        return res.json();
      })
      .then(function (data) {
        if (!data || !data.ok) throw new Error('invalid probe');
        markDone(stepProfile, 1);
        markDone(stepTrust, 2);
        if (statusEl) {
          statusEl.className = 'probe-status ok';
          statusEl.textContent = box.getAttribute('data-ok') || 'OK';
        }
      })
      .catch(function () {
        markPending(stepTrust, 2);
        if (statusEl) {
          statusEl.className = 'probe-status fail';
          statusEl.textContent = box.getAttribute('data-fail') || 'Failed';
        }
      });
  }

  if (btn) btn.addEventListener('click', runProbe);
  document.addEventListener('visibilitychange', function () {
    if (document.visibilityState === 'visible') runProbe();
  });
  runProbe();
})();
</script>"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ios_checklist_renders_both_steps_and_probe_url() {
        for zh in [true, false] {
            let html = install_page_ios_checklist(zh);
            assert!(html.contains("id=\"ios-verify\""));
            assert!(html.contains("id=\"step-profile\""));
            assert!(html.contains("id=\"step-trust\""));
            assert!(html.contains("id=\"ios-probe-btn\""));
            assert!(html.contains(IOS_TLS_PROBE_URL));
        }
        assert!(IOS_PROBE_SCRIPT.contains("visibilitychange"));
        assert!(IOS_PROBE_SCRIPT.contains("https://zniff.local/probe"));
    }
}
