using System;

public class Program {
    public static int Main() {
        // Test 1: Stack-allocated span passed by value
        int[] arr = new int[4];
        arr[0] = 1; arr[1] = 2; arr[2] = 3; arr[3] = 4;
        Span<int> span = new Span<int>(arr);
        
        if (!CheckSpan(span)) {
            Console.WriteLine("CheckSpan failed");
            return 1;
        }
        
        // Test 2: SequenceEqual (calls intrinsic which reads fields from stack)
        Span<int> span2 = new Span<int>(arr);
        if (!span.SequenceEqual(span2)) {
            Console.WriteLine("SequenceEqual failed for identical spans");
            return 2;
        }

        // Test 3: Slice and verify
        var slice = span.Slice(1, 2);
        if (slice.Length != 2 || slice[0] != 2 || slice[1] != 3) {
            Console.WriteLine("Slice failed");
            return 3;
        }
        
        // Test 4: Stackalloc (if supported, otherwise skip)
        // stackalloc produces a span directly on stack, verifying stack-origin pointer handling
        // unsafe {
        //     int* ptr = stackalloc int[3];
        //     ptr[0] = 10; ptr[1] = 20; ptr[2] = 30;
        //     Span<int> stackSpan = new Span<int>(ptr, 3);
        //     if (!CheckSpanStack(stackSpan)) return 4;
        // }
        
        // Test 4: String array (heap allocated)
        string[] sa = new string[2];
        sa[0] = "hello"; sa[1] = "world";
        Span<string> sSpan = new Span<string>(sa);
        if (sSpan.Length != 2) return 5;
        if (sSpan[0] != "hello") return 6;
        if (sSpan[1] != "world") return 7;

        return 0;
    }
    
    static bool CheckSpan(ReadOnlySpan<int> s) {
        if (s.Length != 4) return false;
        if (s[0] != 1) return false;
        if (s[3] != 4) return false;
        return true;
    }

    static bool CheckSpanStack(ReadOnlySpan<int> s) {
        if (s.Length != 3) return false;
        if (s[0] != 10) return false;
        if (s[2] != 30) return false;
        return true;
    }
}
