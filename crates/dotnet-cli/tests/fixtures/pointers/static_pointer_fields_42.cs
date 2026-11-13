using System;
using System.Runtime.InteropServices;

unsafe struct Info
{
    public static Info* s_ptr;
    public static void* s_void;
    public static int* s_int;
    [ThreadStatic] public static Info* t_ptr;
    [ThreadStatic] public static void* t_void;
    public Info* _next;
    public void* _void;
    public int* _int;
}

// Pointer-typed statics hold one native word, like instance pointer fields, and
// must load as native pointers that can be stored into other pointer fields.
public static unsafe class Program
{
    static int failures;

    static void Check(string name, bool ok)
    {
        if (!ok)
        {
            failures++;
            Console.WriteLine("FAIL " + name);
        }
    }

    public static int Main()
    {
        Info* heap = (Info*)NativeMemory.AllocZeroed((nuint)sizeof(Info));
        int value = 7;
        int* local = &value;

        // Null statics (plain and thread-static) into stack struct fields.
        Info l = default;
        l._next = Info.s_ptr;
        l._void = Info.s_void;
        l._int = Info.s_int;
        Check("null s_ptr -> stack", l._next == null && l._void == null && l._int == null);
        l._next = Info.t_ptr;
        l._void = Info.t_ptr;
        l._void = Info.t_void;
        Check("null t_ptr -> stack", l._next == null && l._void == null);

        // Null statics into NativeMemory.
        heap->_next = Info.s_ptr;
        heap->_void = Info.t_void;
        Check("null static -> native", heap->_next == null && heap->_void == null);

        // Non-null round trips.
        Info.s_ptr = heap;
        Info.s_void = heap;
        Info.s_int = local;
        Info.t_ptr = heap;
        Info.t_void = heap;
        l._next = Info.s_ptr;
        l._void = Info.s_void;
        l._int = Info.s_int;
        Check("s_ptr -> stack", l._next == heap && l._void == heap && l._int == local && *l._int == 7);
        l = default;
        l._next = Info.t_ptr;
        l._void = Info.t_void;
        Check("t_ptr -> stack", l._next == heap && l._void == heap);
        heap->_next = Info.s_ptr;
        heap->_void = Info.s_void;
        heap->_int = Info.s_int;
        Check("static -> native", heap->_next == heap && heap->_void == heap && *heap->_int == 7);

        // Back into statics from instance fields, and static to static.
        Info.s_ptr = null;
        Info.t_ptr = null;
        Check("cleared", Info.s_ptr == null && Info.t_ptr == null);
        Info.s_ptr = l._next;
        Info.t_ptr = heap->_next;
        Check("fields -> statics", Info.s_ptr == heap && Info.t_ptr == heap);
        Info.s_void = Info.t_ptr;
        Check("static -> static", Info.s_void == heap);

        Info.s_ptr = null;
        Info.s_void = null;
        Info.s_int = null;
        Info.t_ptr = null;
        Info.t_void = null;
        NativeMemory.Free(heap);
        return failures == 0 ? 42 : 1;
    }
}
