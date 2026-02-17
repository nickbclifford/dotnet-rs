public class Data {
    public int Value = 42;
}
public class Program {
    static Data s_data = new Data();
    public static int Main() {
        // First call will initialize s_data in the current thread's arena.
        // Subsequent calls from other threads will use this object.
        
        // Trigger GC to see if it breaks cross-arena refs
        System.GC.Collect();
        
        return s_data.Value;
    }
}
