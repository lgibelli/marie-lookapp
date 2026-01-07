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

    [DllImport("user32.dll")]
    private static extern bool AttachThreadInput(uint idAttach, uint idAttachTo, bool fAttach);

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);

    [DllImport("kernel32.dll")]
    private static extern uint GetCurrentThreadId();

    private const byte VK_CONTROL = 0x11;
    private const byte VK_V = 0x56;
    private const uint KEYEVENTF_KEYUP = 0x0002;

    // Store the target window before opening search
    private IntPtr _targetWindow = IntPtr.Zero;

    /// <summary>
    /// Call this BEFORE opening the search window to remember where to paste
    /// </summary>
    public void CaptureTargetWindow()
    {
        _targetWindow = GetForegroundWindow();
    }

    /// <summary>
    /// Insert text at the cursor position in the previously captured target window
    /// </summary>
    public void InsertText(string text)
    {
        if (_targetWindow == IntPtr.Zero)
        {
            // Fallback - just copy to clipboard
            Clipboard.SetText(text);
            return;
        }

        try
        {
            // Set text to clipboard
            Clipboard.SetText(text);

            // Force focus back to target window
            ForceForegroundWindow(_targetWindow);

            // Wait for focus to settle
            Thread.Sleep(150);

            // Simulate Ctrl+V
            SimulateCtrlV();
        }
        catch
        {
            // If anything fails, at least the text is in clipboard
        }
        finally
        {
            _targetWindow = IntPtr.Zero;
        }
    }

    private void ForceForegroundWindow(IntPtr hWnd)
    {
        var currentThread = GetCurrentThreadId();
        var targetThread = GetWindowThreadProcessId(hWnd, out _);

        // Attach to target thread to allow SetForegroundWindow
        if (currentThread != targetThread)
        {
            AttachThreadInput(currentThread, targetThread, true);
            SetForegroundWindow(hWnd);
            AttachThreadInput(currentThread, targetThread, false);
        }
        else
        {
            SetForegroundWindow(hWnd);
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
