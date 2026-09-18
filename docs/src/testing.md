# Testing

Current automated tests cover:

- normal in-workspace path resolution
- symlink escape rejection on Unix
- bounded reads

The project should grow toward five layers:

1. unit tests
2. property tests
3. integration tests
4. tunnel end-to-end tests
5. real-machine resource benchmarks

Resource claims for 8 GB Macs require physical-machine evidence rather than CI assumptions.

