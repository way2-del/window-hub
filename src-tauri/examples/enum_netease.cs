using System;
using System.Text;
using System.Runtime.InteropServices;

class P {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc lp, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L,T,R,B; }
  static void Main() {
    EnumWindows((h,l) => {
      var c = new StringBuilder(256);
      GetClassName(h, c, 256);
      var cls = c.ToString();
      if (cls.IndexOf("Lyric", StringComparison.OrdinalIgnoreCase)>=0 || cls.IndexOf("Orpheus", StringComparison.OrdinalIgnoreCase)>=0 || cls.IndexOf("CloudMusic", StringComparison.OrdinalIgnoreCase)>=0 || cls.IndexOf("Desktop", StringComparison.OrdinalIgnoreCase)>=0) {
        var t = new StringBuilder(512);
        GetWindowText(h, t, 512);
        GetWindowRect(h, out var r);
        var vis = IsWindowVisible(h);
        Console.WriteLine($"vis={vis} {r.R-r.L}x{r.B-r.T} class=[{cls}] title=[{t}]");
      }
      return true;
    }, IntPtr.Zero);
  }
}
