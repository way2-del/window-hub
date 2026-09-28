---
name: window-hub-plugin-popup
description: >-
  Window Hub 鎻掍欢鎵樼寮圭獥鍑犱綍 鈥?shell 杈硅窛/闂磋窛 token銆丠ost #app.wg-shell 鎸傝浇绾﹀畾銆?  瀵圭収绐楀彛缁勩€傛敼 entry.popup / popup.css 鎴栨柊鍐欏脊绐楁彃浠舵椂蹇呰銆?---

# 鎻掍欢寮圭獥鍑犱綍锛坋ntry.popup锛?
鎵樼寮圭獥鐢?Host 鎵撳紑锛坄open_plugin_popup`锛夛紝**涓嶆覆鏌撴彃浠剁殑 `popup.html` 澶栧３**锛涘彧娉ㄥ叆鍚岀洰褰?`popup.css` + `popup.js`锛屾寕鍒?Host 鎻愪緵鐨勮妭鐐逛笂銆?
## Host 鎸傝浇绾﹀畾锛堟槗韪╁潙锛?
```tsx
// PluginPopupHost.tsx
<main id="app" className="wg-shell" />
```

| 浜嬪疄 | 鍚箟 |
|------|------|
| 鏍硅妭鐐瑰浐瀹?`#app.wg-shell` | 鎻掍欢 CSS **蹇呴』**鑳藉懡涓?`#app` 鎴?`.wg-shell` |
| `popup.html` 鐨?class 涓嶄細鍑虹幇鍦?DOM | 鍙啓 `.memo-shell` 鑰?Host 鏄?`.wg-shell` 鈫?**杈硅窛鍏ㄩ儴澶辨晥銆佸唴瀹硅创杈?* |
| 鎺ㄨ崘 | 澹虫牱寮忓啓 `#app.wg-shell`锛屾垨鍦?JS 閲?`document.getElementById("app").className = "鈥?` |

鐪熸簮瀵圭収锛歚docs/plugins/examples/window-groups/popup.css` 鈫?`.wg-shell`銆?
## 澶栬竟璺?/ 闂磋窛 token锛堝榻愮獥鍙ｇ粍锛?
| Token | 鍊?| 鐢ㄩ€?|
|-------|-----|------|
| `--wh-popup-pad-x` | **12px** | 宸﹀彸鍐呰竟璺?|
| `--wh-popup-pad-top` | **12px** | 椤跺唴杈硅窛 |
| `--wh-popup-pad-bottom` | **14px** | 搴曞唴杈硅窛锛堢暐澶т簬椤讹紝瑙嗚钀藉簳锛?|
| `--wh-popup-gap` | **10px** | 澹冲唴涓诲尯鍧楀瀭鐩撮棿璺濓紙header / 琛ㄥ崟 / 鍒楄〃锛?|

```css
#app.wg-shell {
  box-sizing: border-box;
  height: 100vh;
  padding: var(--wh-popup-pad-top, 12px) var(--wh-popup-pad-x, 12px)
    var(--wh-popup-pad-bottom, 14px);
  display: flex;
  flex-direction: column;
  gap: var(--wh-popup-gap, 10px);
  overflow: hidden;
}
```

### 纭€ц鍒?
1. **绂佹**鍐呭璐寸獥杈癸細澹充笂蹇呴』鏈変笂琛?padding锛堝彲涓?Host 榛樿鍙犲姞锛屽嬁鍐欐垚 0锛夈€?2. **绂佹**鍙粰鑷畾涔?class 鍐?padding 鍗翠笉鍛戒腑 `#app` / `.wg-shell`銆?3. 鍒楄〃鍖?`flex: 1; min-height: 0; overflow: auto`锛屽嬁璁╁垪琛ㄦ拺鐮村３瀵艰嚧搴曡竟琚銆?4. 宸﹀彸瀵圭О锛涘簳 鈮?椤讹紙榛樿 14 / 12锛夈€?5. 鍖哄潡闂磋窛鐢?`gap: 10px`锛屼笉瑕侀潬璐?margin 椤惰竟銆?
## 澹冲唴娆＄骇闂磋窛锛堝缓璁級

| 鍏冪礌 | 寤鸿 |
|------|------|
| 鏍囬 | 13px / weight 650 |
| 杈呭姪璇存槑 | 11px銆乵uted |
| 涓绘寜閽珮 | 28鈥?2px |
| 鍒楄〃琛屽唴杈硅窛 | 鈮?9鈥?0px |
| 琛ㄥ崟鎺т欢鍦嗚 | 6鈥?px |

绐楀彛灏哄榛樿 Host **320脳480**锛坄PLUGIN_POPUP_W/H`锛夛紱鎻掍欢鍙敤 `settings.popupWidth` / `popupHeight` 鎴?`hub.popup.open({ width, height })` 鑷畾涔夛紙clamp锛氬 **280鈥?400**銆侀珮 **320鈥?600**锛夈€傚唴瀹规寜绐楀甯冨眬锛屽嬁鍋囧畾鍥哄畾鍍忕礌銆?
澶х敾甯冪被鎻掍欢鍙澶栦娇鐢細

| API | 璇存槑 |
|-----|------|
| `hub.popup.open({ nativeFrame, resizable, windowedFullscreen })` | `nativeFrame: true` 鈫?绯荤粺鏍囬鏍忥紙鍚岃缃獥锛夛紱鏈€澶у寲鐢ㄧ郴缁熸寜閽垨 `windowedFullscreen` |
| `hub.popup.setWindowedFullscreen(bool)` | 鍘熺敓绐?maximize锛涙棤杈规鍒欓摵婊″伐浣滃尯 |
| `hub.popup.resize({ width, height })` | 璋冩暣宸插紑寮圭獥 |

瀹樻柟绀轰緥锛歚docs/plugins/examples/excalidraw/`锛堝缁?`nativeFrame: true`锛夈€?
## 鑷

- [ ] 鎵撳紑寮圭獥鍚庯紝鍥涜竟鍙绌洪殭锛堢害 12px锛?- [ ] 寮€鍙戣€呭伐鍏烽噷 `#app` 鐨?computed padding 闈?0
- [ ] class 鍛戒腑 `.wg-shell` 鎴栧凡鍦?JS 閲嶈 class
- [ ] 鏃?`alert` / `confirm` / `prompt`

## Related

- 鎬绘祦绋嬶細`window-hub-plugin`
- 瀹樻柟瀵圭収锛歚window-hub-window-groups`
