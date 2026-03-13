using System;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

namespace System.Runtime.InteropServices {
    public enum GCHandleType {
        Weak = 0,
        WeakTrackResurrection = 1,
        Normal = 2,
        Pinned = 3
    }

    public struct GCHandle {
        private IntPtr _handle;

        public static GCHandle Alloc(object value, GCHandleType type) {
            GCHandle h = new GCHandle();
            h._handle = InternalAlloc(value, type);
            return h;
        }

        public void Free() {
            InternalFree(_handle);
        }

        public object Target {
            get => InternalGet(_handle);
            set => InternalSet(_handle, value);
        }

        public bool IsAllocated => _handle != IntPtr.Zero;

        [MethodImpl(MethodImplOptions.InternalCall)]
        private static extern IntPtr InternalAlloc(object value, GCHandleType type);

        [MethodImpl(MethodImplOptions.InternalCall)]
        private static extern void InternalFree(IntPtr handle);

        [MethodImpl(MethodImplOptions.InternalCall)]
        private static extern object InternalGet(IntPtr handle);

        [MethodImpl(MethodImplOptions.InternalCall)]
        private static extern void InternalSet(IntPtr handle, object value);
    }
}

public class Program {
    public static object Target;
    
    public static GCHandle CreateWeak() {
        Target = new object();
        GCHandle h = GCHandle.Alloc(Target, GCHandleType.Weak);
        Target = null;
        return h;
    }

    public static int Main() {
        GCHandle h = CreateWeak();
        
        GC.Collect();
        GC.WaitForPendingFinalizers();
        
        // After GC, it should be dead
        if (h.Target != null) {
            return 1; // Object should have been collected
        }
        
        h.Free();
        return 42;
    }
}
