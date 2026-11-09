cmake_minimum_required(VERSION 3.16)
project(spotcore LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)

find_package(Qt6 REQUIRED COMPONENTS Core Gui Widgets Network WebEngineCore WebEngineWidgets)

add_executable(spotcore-qt-engine main.cpp)
target_link_libraries(spotcore-qt-engine PRIVATE
    Qt6::Core Qt6::Gui Qt6::Widgets Qt6::Network Qt6::WebEngineCore Qt6::WebEngineWidgets)
