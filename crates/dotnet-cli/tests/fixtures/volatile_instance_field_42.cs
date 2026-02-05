public class VolatileTest {
    public volatile int Flag = 0;
    public int Data = 0;
}

public class Program {
    public static int Main() {
        VolatileTest t = new VolatileTest();
        t.Data = 42;
        t.Flag = 1;
        
        if (t.Flag == 1 && t.Data == 42) {
            return 42;
        }
        return 0;
    }
}
