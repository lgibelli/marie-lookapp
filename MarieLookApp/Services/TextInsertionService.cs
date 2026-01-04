using System.Runtime.InteropServices;
using System.Windows;

namespace MarieLookApp.Services;

public class TextInsertionService
{
    [DllImport("user32.dll")]
    private static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);

    [DllImport("user32.dll")]
    private static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll")]
    private static extern bool SetForegroundWindow(IntPtr hWnd);

    private const byte VK_CONTROL = 0x11;
    private const byte VK_V = 0x56;
    private const uint KEYEVENTF_KEYUP = 0x0002;

    /// <summary>
    /// Insert text at the current cursor position by copying to clipboard
    /// and simulating Ctrl+V
    /// </summary>
    public void InsertText(string text)
    {
        // Store original clipboard content
        string? originalClipboard = null;
        try
        {
            if (Clipboard.ContainsText())
            {
                originalClipboard = Clipboard.GetText();
            }
        }
        catch { }

        try
        {
            // Set our text to clipboard
            Clipboard.SetText(text);

            // Ensure the foreground window has focus before sending keys
            var foreground = GetForegroundWindow();
            if (foreground != IntPtr.Zero)
            {
                SetForegroundWindow(foreground);
            }

            // Delay to ensure clipboard and focus are ready
            Thread.Sleep(100);

            // Simulate Ctrl+V
            SimulateCtrlV();

            // Small delay before restoring clipboard
            Thread.Sleep(150);
        }
        finally
        {
            // Restore original clipboard content after a delay
            if (originalClipboard != null)
            {
                Task.Delay(500).ContinueWith(_ =>
                {
                    try
                    {
                        Application.Current?.Dispatcher.Invoke(() =>
                        {
                            try { Clipboard.SetText(originalClipboard); } catch { }
                        });
                    }
                    catch { }
                });
            }
        }
    }

    private static void SimulateCtrlV()
    {
        // Press Ctrl
        keybd_event(VK_CONTROL, 0, 0, UIntPtr.Zero);
        // Press V
        keybd_event(VK_V, 0, 0, UIntPtr.Zero);
        // Release V
        keybd_event(VK_V, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
        // Release Ctrl
        keybd_event(VK_CONTROL, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
    }
}
