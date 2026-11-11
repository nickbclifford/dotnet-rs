using System;
using System.Runtime.InteropServices;

public static class Program
{
    [DllImport("kernel32.dll", EntryPoint = "CreateEventEx")]
    private static extern IntPtr CreateEventEx(IntPtr attributes, IntPtr name, uint flags, uint access);

    [DllImport("libSystem.Native", EntryPoint = "SystemNative_LowLevelMonitor_Create")]
    private static extern IntPtr LowLevelMonitorCreate();

    [DllImport("libSystem.Native", EntryPoint = "SystemNative_LowLevelMonitor_Destroy")]
    private static extern void LowLevelMonitorDestroy(IntPtr monitor);

    [DllImport("System.Private.CoreLib", EntryPoint = "WaitHandle_WaitOneCore")]
    private static extern int WaitHandleWaitOneCore(IntPtr handle, int timeout, bool alertable);

    public static int Main()
    {
        try { _ = CreateEventEx(IntPtr.Zero, IntPtr.Zero, 0, 0); }
        catch (DllNotFoundException) { }
        catch (EntryPointNotFoundException) { }

        IntPtr monitor = LowLevelMonitorCreate();
        if (monitor != IntPtr.Zero) LowLevelMonitorDestroy(monitor);

        try { _ = WaitHandleWaitOneCore(IntPtr.Zero, 0, false); }
        catch (DllNotFoundException) { }
        catch (EntryPointNotFoundException) { }

        return 42;
    }
}
