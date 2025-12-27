using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;

namespace MarieLookApp.Services;

public class HotkeyService : IDisposable
{
    private const int WM_HOTKEY = 0x0312;
    private const int HOTKEY_ID = 9000;

    // Modifier keys
    private const uint MOD_CONTROL = 0x0002;
    private const uint MOD_SHIFT = 0x0004;
    private const uint MOD_NOREPEAT = 0x4000;

    // Virtual key code for 'L'
    private const uint VK_L = 0x4C;

    [DllImport("user32.dll")]
    private static extern bool RegisterHotKey(IntPtr hWnd, int id, uint fsModifiers, uint vk);

    [DllImport("user32.dll")]
    private static extern bool UnregisterHotKey(IntPtr hWnd, int id);

    private HwndSource? _source;
    private IntPtr _windowHandle;
    private bool _isRegistered;

    public event Action? HotkeyPressed;

    public void Register(Window window)
    {
        var helper = new WindowInteropHelper(window);
        helper.EnsureHandle();
        _windowHandle = helper.Handle;

        _source = HwndSource.FromHwnd(_windowHandle);
        _source?.AddHook(HwndHook);

        // Register Ctrl+Shift+L
        _isRegistered = RegisterHotKey(_windowHandle, HOTKEY_ID, MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT, VK_L);

        if (!_isRegistered)
        {
            System.Diagnostics.Debug.WriteLine("Failed to register hotkey Ctrl+Shift+L");
        }
    }

    private IntPtr HwndHook(IntPtr hwnd, int msg, IntPtr wParam, IntPtr lParam, ref bool handled)
    {
        if (msg == WM_HOTKEY && wParam.ToInt32() == HOTKEY_ID)
        {
            HotkeyPressed?.Invoke();
            handled = true;
        }
        return IntPtr.Zero;
    }

    public void Dispose()
    {
        if (_isRegistered && _windowHandle != IntPtr.Zero)
        {
            UnregisterHotKey(_windowHandle, HOTKEY_ID);
        }
        _source?.RemoveHook(HwndHook);
        _source?.Dispose();
    }
}
