using System;

class Payload {
    public int id;
    public Payload(int i) { id = i; }
}

struct Mixed {
    public int a;       // 4 bytes, padded to 8? alignment depends.
    public Payload p1;  // 8 bytes (ref)
    public long b;      // 8 bytes
    public Payload p2;  // 8 bytes (ref)
    public byte c;      // 1 byte
}

class Holder {
    public Mixed m;
}

public class Program {
    public static int Main() {
        Holder h = new Holder();
        h.m.a = 123;
        h.m.p1 = new Payload(1);
        h.m.b = 456;
        h.m.p2 = new Payload(2);
        h.m.c = 7;

        // Force GC by allocating a lot. 
        // We rely on the VM's GC trigger mechanism (allocation pressure).
        for(int i=0; i<1000; i++) {
            object[] arr = new object[100];
        }

        if (h.m.p1.id != 1) return 1;
        if (h.m.p2.id != 2) return 2;
        
        return 0;
    }
}
