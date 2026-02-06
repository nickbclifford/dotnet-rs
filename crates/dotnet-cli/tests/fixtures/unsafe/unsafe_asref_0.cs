using System;
using System.Runtime.CompilerServices;

class Program {
    struct TestStruct {
        public int A;
        public int B;
    }

    class TestClass {
        public int A;
    }

    static unsafe void Main() {
        // Test 1: Stack allocation
        TestStruct s = new TestStruct();
        s.A = 123;
        s.B = 456;
        
        void* p = Unsafe.AsPointer(ref s);
        ref TestStruct r = ref Unsafe.AsRef<TestStruct>(p);
        
        if (r.A != 123) throw new Exception("Failed stack AsPointer/AsRef A");
        if (r.B != 456) throw new Exception("Failed stack AsPointer/AsRef B");
    }
}
