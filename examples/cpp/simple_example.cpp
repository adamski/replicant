#include "sync_client.hpp"
#include <iostream>
#include <string>

int main()
{
    try
    {
        // Create sync engine with HMAC authentication
        SyncClient::sync_engine engine(
            "sqlite:client.db?mode=rwc",
            "ws://localhost:8080/ws",
            "test-user@example.com",
            "rpa_test_api_key_example_12345",
            "rps_test_api_secret_example_67890"
        );
        
        std::cout << "Sync client version: " << SyncClient::sync_engine::get_version() << std::endl;

        // Create a document
        std::string doc_id = engine.create_document(
            R"({"title":"My Document","content":"Hello World","type":"note","priority":"medium"})"
        );
        
        std::cout << "Created document: " << doc_id << std::endl;
        
        // Update the document
        engine.update_document(
            doc_id,
            R"({"content":"Hello Updated World","type":"note","priority":"high"})"
        );
        
        std::cout << "Updated document successfully" << std::endl;
        
        // Delete the document
        engine.delete_document(doc_id);
        std::cout << "Deleted document successfully" << std::endl;
        
    } 
    catch (const SyncClient::sync_exception& e) 
    {
        std::cerr << "Sync error: " << e.what() << std::endl;
        return 1;
    } 
    catch (const std::exception& e) 
    {
        std::cerr << "Error: " << e.what() << std::endl;
        return 1;
    }
    
    return 0;
}