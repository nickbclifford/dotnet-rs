using System;

public sealed class HeapCell {
    public int Field;
}

public class Program {
    private const int Length = 512;
    private const int Repetitions = 128;

    public static unsafe int Main() {
        int[] arr = new int[Length];
        for (int i = 0; i < arr.Length; i++) {
            arr[i] = i;
        }

        // Managed byrefs to array elements (ldelema followed by indirect access).
        for (int pass = 0; pass < Repetitions; pass++) {
            for (int i = 0; i < arr.Length; i++) {
                ref int element = ref arr[i];
                element += 1;
            }
        }

        // Address and update a field on a heap-allocated object.
        HeapCell obj = new HeapCell();
        for (int pass = 0; pass < Repetitions; pass++) {
            for (int i = 0; i < Length; i++) {
                ref int field = ref obj.Field;
                field += 1;
            }
        }

        // Pin each array-element address and update through its native pointer.
        for (int pass = 0; pass < Repetitions; pass++) {
            for (int i = 0; i < arr.Length; i++) {
                fixed (int* element = &arr[i]) {
                    *element += 1;
                }
            }
        }

        // Convert the pinned heap array address to a native pointer, then use
        // pointer arithmetic and unmanaged indirect reads/writes within bounds.
        fixed (int* basePtr = arr) {
            for (int pass = 0; pass < Repetitions; pass++) {
                for (int i = 0; i < arr.Length; i++) {
                    int* element = basePtr + i;
                    *element += 1;
                }
            }
        }

        // Exercise Span<T>'s indexed element access in a separate loop.
        Span<int> span = arr;
        for (int pass = 0; pass < Repetitions; pass++) {
            for (int i = 0; i < span.Length; i++) {
                span[i] += 1;
            }
        }

        int expectedArrayStart = Repetitions * 4;
        int expectedArrayEnd = Length - 1 + expectedArrayStart;
        int expectedField = Repetitions * Length;
        return arr[0] == expectedArrayStart
            && arr[Length - 1] == expectedArrayEnd
            && obj.Field == expectedField
            ? 0
            : 1;
    }
}
