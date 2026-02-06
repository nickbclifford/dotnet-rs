unsafe class Program {
    static int Main() {
        object[] arr = new object[1];
        // This should throw IndexOutOfRangeException
        // The runtime must not crash/deadlock during the throw.
        try {
            SetVal(arr, 5, "test");
            return 1; // Failed: Did not throw
        } catch (System.IndexOutOfRangeException) {
            return 42; // Success: Caught expected exception
        } catch (System.Exception) {
             return 2; // Failed: Caught unexpected exception
        }
    }

    static void SetVal(object[] arr, int idx, object val) {
        // This assignment uses stelem.ref which was the source of the reentrancy bug
        arr[idx] = val; 
    }
}
