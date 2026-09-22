// Copyright 2026 sunaemon
// SPDX-License-Identifier: MIT

using System;
using System.Diagnostics;
using System.Drawing;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;

namespace KeymapOverlay.Hil
{
    public static class WindowsUiProbe
    {
        private const int GwlExstyle = -20;
        private const long WsExNoactivate = 0x08000000L;
        private const long WsExToolwindow = 0x00000080L;
        private const long WsExTopmost = 0x00000008L;
        private const long WsExTransparent = 0x00000020L;
        private const uint InputMouse = 0;
        private const uint MouseeventfLeftdown = 0x0002;
        private const uint MouseeventfLeftup = 0x0004;

        public static string Run(
            string driver,
            int overlayProcessId,
            int layerKeyboardId,
            int layer,
            int inputKeyboardId,
            int encoderIndex)
        {
            if (Thread.CurrentThread.GetApartmentState() != ApartmentState.STA)
            {
                throw new InvalidOperationException("The Windows HIL UI probe requires an STA thread");
            }

            var keyDownCount = 0;
            var keyUpCount = 0;
            var clickCount = 0;
            var form = new Form
            {
                Bounds = SystemInformation.VirtualScreen,
                FormBorderStyle = FormBorderStyle.None,
                ShowInTaskbar = false,
                StartPosition = FormStartPosition.Manual,
                Text = "Keymap Overlay HIL Input Target",
            };
            var editor = new TextBox
            {
                Location = new Point(20, 20),
                Width = 480,
                Text = "Windows HIL input target",
            };
            editor.KeyDown += delegate(object sender, KeyEventArgs eventArgs)
            {
                if (eventArgs.KeyCode == Keys.A)
                {
                    keyDownCount += 1;
                }
            };
            editor.KeyUp += delegate(object sender, KeyEventArgs eventArgs)
            {
                if (eventArgs.KeyCode == Keys.A)
                {
                    keyUpCount += 1;
                }
            };
            form.MouseDown += delegate { clickCount += 1; };
            form.Controls.Add(editor);

            var cursor = new NativeMethods.Point();
            NativeMethods.GetCursorPos(out cursor);
            try
            {
                form.Show();
                form.Activate();
                editor.Focus();
                PumpFor(250);
                AssertFocus(form, editor, "before the overlay was shown");

                Rotate(driver, inputKeyboardId, encoderIndex);
                WaitFor(delegate { return keyDownCount == 1 && keyUpCount == 1; },
                    "the pre-overlay standard key event");

                SendLayer(driver, layerKeyboardId, layer, "press");
                var overlay = WaitForVisibleOverlay(overlayProcessId);
                AssertWindowStyles(overlay);
                AssertFocus(form, editor, "after the first overlay show");
                Rotate(driver, inputKeyboardId, encoderIndex);
                WaitFor(delegate { return keyDownCount == 2 && keyUpCount == 2; },
                    "the first-show standard key event");
                SendLayer(driver, layerKeyboardId, layer, "release");
                WaitForHiddenOverlay(overlayProcessId);

                SendLayer(driver, layerKeyboardId, layer, "press");
                overlay = WaitForVisibleOverlay(overlayProcessId);
                AssertFocus(form, editor, "after the second overlay show");
                Rotate(driver, inputKeyboardId, encoderIndex);
                WaitFor(delegate { return keyDownCount == 3 && keyUpCount == 3; },
                    "the second-show standard key event");

                var bounds = GetWindowBounds(overlay);
                var clickPoint = new Point(
                    bounds.Left + (bounds.Right - bounds.Left) / 2,
                    bounds.Top + (bounds.Bottom - bounds.Top) / 2);
                NativeMethods.SetCursorPos(clickPoint.X, clickPoint.Y);
                SendLeftClick();
                WaitFor(delegate { return clickCount == 1; }, "the click-through event");

                SendLayer(driver, layerKeyboardId, layer, "release");
                WaitForHiddenOverlay(overlayProcessId);
                editor.Focus();
                PumpFor(100);
                Rotate(driver, inputKeyboardId, encoderIndex);
                WaitFor(delegate { return keyDownCount == 4 && keyUpCount == 4; },
                    "the post-overlay standard key event");

                return "PASS: Windows HIL focus, standard key input, click-through, topmost, and repeated show";
            }
            finally
            {
                TrySendLayerRelease(driver, layerKeyboardId, layer);
                NativeMethods.SetCursorPos(cursor.X, cursor.Y);
                form.Close();
                form.Dispose();
            }
        }

        private static void SendLayer(
            string driver,
            int keyboardId,
            int layer,
            string state)
        {
            RunDriver(
                driver,
                "layer --keyboard-id " + keyboardId + " --layer " + layer + " --state " + state);
            PumpFor(150);
        }

        private static void TrySendLayerRelease(string driver, int keyboardId, int layer)
        {
            try
            {
                SendLayer(driver, keyboardId, layer, "release");
            }
            catch
            {
                // The PowerShell session cleanup makes another independent release attempt.
            }
        }

        private static void Rotate(string driver, int keyboardId, int encoderIndex)
        {
            RunDriver(
                driver,
                "rotate --keyboard-id " + keyboardId + " --index " + encoderIndex
                    + " --direction ccw");
            PumpFor(150);
        }

        private static void RunDriver(string driver, string arguments)
        {
            var startInfo = new ProcessStartInfo
            {
                Arguments = arguments,
                CreateNoWindow = true,
                FileName = driver,
                RedirectStandardError = true,
                RedirectStandardOutput = true,
                UseShellExecute = false,
            };
            using (var process = Process.Start(startInfo))
            {
                var output = process.StandardOutput.ReadToEnd();
                var error = process.StandardError.ReadToEnd();
                process.WaitForExit();
                if (process.ExitCode != 0)
                {
                    throw new InvalidOperationException(
                        "HIL driver failed: " + arguments + Environment.NewLine + output + error);
                }
            }
        }

        private static IntPtr WaitForVisibleOverlay(int processId)
        {
            IntPtr result = IntPtr.Zero;
            WaitFor(delegate
            {
                result = FindWindow(processId);
                if (result == IntPtr.Zero || !NativeMethods.IsWindowVisible(result))
                {
                    return false;
                }
                var bounds = GetWindowBounds(result);
                return bounds.Right - bounds.Left > 1 && bounds.Bottom - bounds.Top > 1;
            }, "the visible Win32 overlay");
            return result;
        }

        private static void WaitForHiddenOverlay(int processId)
        {
            WaitFor(delegate
            {
                var window = FindWindow(processId);
                return window == IntPtr.Zero || !NativeMethods.IsWindowVisible(window);
            }, "the hidden Win32 overlay");
        }

        private static IntPtr FindWindow(int processId)
        {
            var result = IntPtr.Zero;
            NativeMethods.EnumWindows(delegate(IntPtr window, IntPtr parameter)
            {
                int ownerProcessId;
                NativeMethods.GetWindowThreadProcessId(window, out ownerProcessId);
                if (ownerProcessId == processId)
                {
                    result = window;
                    return false;
                }
                return true;
            }, IntPtr.Zero);
            return result;
        }

        private static NativeMethods.Rect GetWindowBounds(IntPtr window)
        {
            NativeMethods.Rect bounds;
            if (!NativeMethods.GetWindowRect(window, out bounds))
            {
                throw new InvalidOperationException("Could not read the Win32 overlay bounds");
            }
            return bounds;
        }

        private static void AssertWindowStyles(IntPtr overlay)
        {
            var styles = NativeMethods.GetWindowLongPtr(overlay, GwlExstyle).ToInt64();
            var expected = WsExNoactivate | WsExToolwindow | WsExTopmost | WsExTransparent;
            if ((styles & expected) != expected)
            {
                throw new InvalidOperationException(
                    "The visible overlay is missing required extended window styles");
            }
        }

        private static void AssertFocus(Form form, TextBox editor, string description)
        {
            PumpFor(100);
            if (NativeMethods.GetForegroundWindow() != form.Handle || !editor.Focused)
            {
                throw new InvalidOperationException("The overlay moved input focus " + description);
            }
        }

        private static void SendLeftClick()
        {
            var inputs = new[]
            {
                NativeMethods.Input.Mouse(MouseeventfLeftdown),
                NativeMethods.Input.Mouse(MouseeventfLeftup),
            };
            var sent = NativeMethods.SendInput(
                (uint)inputs.Length,
                inputs,
                Marshal.SizeOf(typeof(NativeMethods.Input)));
            if (sent != inputs.Length)
            {
                throw new InvalidOperationException("Could not inject the HIL click-through probe");
            }
        }

        private static void WaitFor(Func<bool> predicate, string description)
        {
            var deadline = DateTime.UtcNow.AddSeconds(5);
            while (DateTime.UtcNow < deadline)
            {
                Application.DoEvents();
                if (predicate())
                {
                    return;
                }
                Thread.Sleep(20);
            }
            throw new InvalidOperationException("Timed out waiting for " + description);
        }

        private static void PumpFor(int milliseconds)
        {
            var deadline = DateTime.UtcNow.AddMilliseconds(milliseconds);
            while (DateTime.UtcNow < deadline)
            {
                Application.DoEvents();
                Thread.Sleep(10);
            }
        }

        private static class NativeMethods
        {
            internal delegate bool EnumWindowsCallback(IntPtr window, IntPtr parameter);

            [StructLayout(LayoutKind.Sequential)]
            internal struct Point
            {
                internal int X;
                internal int Y;
            }

            [StructLayout(LayoutKind.Sequential)]
            internal struct Rect
            {
                internal int Left;
                internal int Top;
                internal int Right;
                internal int Bottom;
            }

            [StructLayout(LayoutKind.Sequential)]
            internal struct MouseInput
            {
                internal int Dx;
                internal int Dy;
                internal uint MouseData;
                internal uint Flags;
                internal uint Time;
                internal IntPtr ExtraInfo;
            }

            [StructLayout(LayoutKind.Explicit)]
            internal struct InputUnion
            {
                [FieldOffset(0)]
                internal MouseInput Mouse;
            }

            [StructLayout(LayoutKind.Sequential)]
            internal struct Input
            {
                internal uint Type;
                internal InputUnion Union;

                internal static Input Mouse(uint flags)
                {
                    return new Input
                    {
                        Type = InputMouse,
                        Union = new InputUnion
                        {
                            Mouse = new MouseInput { Flags = flags },
                        },
                    };
                }
            }

            [DllImport("user32.dll")]
            [return: MarshalAs(UnmanagedType.Bool)]
            internal static extern bool EnumWindows(EnumWindowsCallback callback, IntPtr parameter);

            [DllImport("user32.dll")]
            internal static extern IntPtr GetForegroundWindow();

            [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")]
            internal static extern IntPtr GetWindowLongPtr(IntPtr window, int index);

            [DllImport("user32.dll")]
            [return: MarshalAs(UnmanagedType.Bool)]
            internal static extern bool GetWindowRect(IntPtr window, out Rect bounds);

            [DllImport("user32.dll")]
            internal static extern uint GetWindowThreadProcessId(
                IntPtr window,
                out int processId);

            [DllImport("user32.dll")]
            [return: MarshalAs(UnmanagedType.Bool)]
            internal static extern bool IsWindowVisible(IntPtr window);

            [DllImport("user32.dll")]
            [return: MarshalAs(UnmanagedType.Bool)]
            internal static extern bool GetCursorPos(out Point point);

            [DllImport("user32.dll")]
            internal static extern uint SendInput(
                uint inputCount,
                [In] Input[] inputs,
                int inputSize);

            [DllImport("user32.dll")]
            [return: MarshalAs(UnmanagedType.Bool)]
            internal static extern bool SetCursorPos(int x, int y);
        }
    }
}
